use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use salvo::http::Method;
use salvo::http::cookie::{Cookie, SameSite, time::Duration};
use salvo::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::{
    schools,
    templates::{PageView, is_htmx},
};

mod views;

use views::CredentialsForm;

const SESSION_COOKIE: &str = "bound_session";

#[derive(Clone)]
pub struct Auth {
    origin: String,
    mailer: crate::mail::Mailer,
}

impl Auth {
    pub fn from_env() -> Self {
        Self {
            origin: std::env::var("APP_URL")
                .unwrap_or_else(|_| "http://localhost:8000".into())
                .trim_end_matches('/')
                .to_owned(),
            mailer: crate::mail::Mailer::from_env(),
        }
    }

    fn cookie(&self, token: String) -> Cookie<'static> {
        Cookie::build((SESSION_COOKIE, token))
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(self.origin.starts_with("https://"))
            .path("/")
            .max_age(Duration::days(30))
            .build()
    }
}

pub fn routes() -> Router {
    Router::with_path("auth")
        .hoop(protect)
        .push(Router::with_path("signup").get(signup_form).post(signup))
        .push(Router::with_path("signin").get(signin_form).post(signin))
        .push(Router::with_path("signout").post(signout))
        .push(Router::with_path("verification").get(verification_page))
        .push(Router::with_path("verification/status").get(verification_status))
        .push(
            Router::with_path("verify")
                .get(verification_form)
                .post(verify_email),
        )
        .push(Router::with_path("resend").post(resend_verification))
}

#[derive(sqlx::FromRow)]
pub struct CurrentUser {
    pub id: i64,
    pub email: String,
    pub is_admin: bool,
    email_verified: bool,
}

impl CurrentUser {
    pub(crate) fn verified_school(&self) -> Option<&'static schools::School> {
        schools::from_email(&self.email).filter(|_| self.email_verified)
    }
}

async fn current_user(req: &Request, depot: &Depot) -> Result<Option<CurrentUser>, StatusError> {
    let Some(cookie) = req.cookie(SESSION_COOKIE) else {
        return Ok(None);
    };
    sqlx::query_as(
        "SELECT users.id, users.email, users.is_admin,
         users.email_verified_at IS NOT NULL AS email_verified FROM sessions
         JOIN users ON users.id = sessions.user_id
         WHERE sessions.token_hash = $1 AND sessions.expires_at > now()",
    )
    .bind(token_hash(cookie.value()))
    .fetch_optional(depot.get_typed::<PgPool>().unwrap())
    .await
    .map_err(internal)
}

/// Require a verified school account and make CurrentUser available to handlers.
#[handler]
pub(crate) async fn require_verified_user(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    match current_user(req, depot).await {
        Ok(Some(user)) if user.verified_school().is_some() => {
            depot.insert_typed(user);
            return;
        }
        Ok(Some(_)) => redirect(req, res, "/auth/verification"),
        Ok(None) => redirect(req, res, "/auth/signin"),
        Err(error) => res.render(error),
    }
    ctrl.skip_rest();
}

/// Apply to admin routers; handlers can read CurrentUser from Depot.
#[handler]
pub async fn require_admin(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    match current_user(req, depot).await {
        Ok(Some(user)) if user.verified_school().is_none() => {
            redirect(req, res, "/auth/verification")
        }
        Ok(Some(user)) if user.is_admin => {
            depot.insert_typed(user);
            return;
        }
        Ok(Some(_)) => {
            res.render(StatusError::forbidden());
        }
        Ok(None) => redirect(req, res, "/auth/signin"),
        Err(error) => {
            res.render(error);
        }
    }
    ctrl.skip_rest();
}

fn redirect(req: &Request, res: &mut Response, path: &'static str) {
    if is_htmx(req) {
        res.status_code(StatusCode::OK);
        res.headers_mut()
            .insert("hx-redirect", path.parse().unwrap());
    } else {
        res.render(Redirect::other(path));
    }
}

fn signup_error(req: &Request, res: &mut Response, status: StatusCode, form: CredentialsForm<'_>) {
    res.status_code(status);
    views::Signup::new(form).show(req, res);
}

#[handler]
pub(crate) async fn protect(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
    ctrl: &mut FlowCtrl,
) {
    res.headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    res.headers_mut()
        .insert("vary", "HX-Request".parse().unwrap());
    res.headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    if req.method() == Method::POST {
        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
        if origin != Some(depot.get_typed::<Auth>().unwrap().origin.as_str()) {
            res.render(StatusError::forbidden());
            ctrl.skip_rest();
        }
    }
}

#[handler]
async fn signup_form(req: &mut Request, res: &mut Response) {
    views::Signup::new(CredentialsForm::default()).show(req, res);
}

#[handler]
async fn signin_form(req: &mut Request, res: &mut Response) {
    views::Signin {
        form: CredentialsForm::default(),
    }
    .show(req, res);
}

fn normalize_email(email: &str) -> Option<String> {
    let email = email.trim().to_lowercase();
    let _: lettre::Address = email.parse().ok()?;
    (email.len() <= 254).then_some(email)
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS randomness unavailable");
    hex::encode(bytes)
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn password_hasher() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
}

fn hash_password(password: &str) -> String {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("OS randomness unavailable");
    password_hasher()
        .hash_password(password.as_bytes(), &SaltString::encode_b64(&salt).unwrap())
        .expect("Failed to hash password")
        .to_string()
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        password_hasher()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

fn internal(error: impl std::fmt::Display) -> StatusError {
    tracing::error!(%error, "Authentication operation failed");
    StatusError::internal_server_error()
}

async fn start_session(
    pool: &PgPool,
    user_id: i64,
    req: &Request,
    auth: &Auth,
    res: &mut Response,
) -> Result<(), StatusError> {
    let token = new_token();
    let mut transaction = pool.begin().await.map_err(internal)?;
    if let Some(old) = req.cookie(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash(old.value()))
            .execute(&mut *transaction)
            .await
            .map_err(internal)?;
    }
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '30 days')")
        .bind(token_hash(&token)).bind(user_id).execute(&mut *transaction).await.map_err(internal)?;
    transaction.commit().await.map_err(internal)?;
    res.add_cookie(auth.cookie(token));
    Ok(())
}

#[handler]
async fn signup(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let submitted_email = req.form::<String>("email").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    let Some(email) = normalize_email(&submitted_email) else {
        signup_error(
            req,
            res,
            StatusCode::BAD_REQUEST,
            CredentialsForm {
                email: &submitted_email,
                email_error: "Please enter a valid school email address.",
                ..Default::default()
            },
        );
        return;
    };
    let Some(school) = schools::from_email(&email) else {
        signup_error(
            req,
            res,
            StatusCode::BAD_REQUEST,
            CredentialsForm {
                email: &submitted_email,
                email_error: &format!(
                    "Please use an officially supported school email address. Supported domains: {}.",
                    schools::supported_domains()
                ),
                ..Default::default()
            },
        );
        return;
    };
    if !(12..=128).contains(&password.chars().count()) {
        signup_error(
            req,
            res,
            StatusCode::BAD_REQUEST,
            CredentialsForm {
                email: &submitted_email,
                password_error: "Use a password between 12 and 128 characters.",
                ..Default::default()
            },
        );
        return;
    }
    match create_account(req, depot, res, &email, password, school).await {
        Ok(SignupOutcome::Created) => redirect(req, res, "/auth/verification"),
        Ok(SignupOutcome::AlreadyExists) => signup_error(
            req,
            res,
            StatusCode::CONFLICT,
            CredentialsForm {
                email: &submitted_email,
                email_error: "Unable to create this account. Try signing in with this email instead.",
                ..Default::default()
            },
        ),
        Ok(SignupOutcome::DeliveryFailed) => {
            res.status_code(StatusCode::SERVICE_UNAVAILABLE);
            views::Verification {
                email: &email,
                pending: false,
                error: "Your account was created, but we couldn't send the verification email. Please try resending it.",
            }.show(req, res);
        }
        Err(_) => signup_error(
            req,
            res,
            StatusCode::INTERNAL_SERVER_ERROR,
            CredentialsForm {
                email: &submitted_email,
                error: "We couldn't complete your signup right now. Please try again, or sign in if you've already registered.",
                ..Default::default()
            },
        ),
    }
}

enum SignupOutcome {
    Created,
    AlreadyExists,
    DeliveryFailed,
}

async fn create_account(
    req: &Request,
    depot: &Depot,
    res: &mut Response,
    email: &str,
    password: String,
    school: &schools::School,
) -> Result<SignupOutcome, StatusError> {
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(internal)?;
    let pool = depot.get_typed::<PgPool>().unwrap();
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2) ON CONFLICT (email) DO NOTHING RETURNING id")
        .bind(email).bind(hash).fetch_optional(pool).await.map_err(internal)?;
    let Some(id) = id else {
        return Ok(SignupOutcome::AlreadyExists);
    };
    let auth = depot.get_typed::<Auth>().unwrap();
    start_session(pool, id, req, auth, res).await?;
    if send_verification(pool, auth, id, email, school)
        .await
        .is_err()
    {
        Ok(SignupOutcome::DeliveryFailed)
    } else {
        Ok(SignupOutcome::Created)
    }
}

#[handler]
async fn signin(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let submitted_email = req.form::<String>("email").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    let email =
        normalize_email(&submitted_email).filter(|email| schools::from_email(email).is_some());
    let Some(email) = email.filter(|_| password.chars().count() <= 128) else {
        res.status_code(StatusCode::UNAUTHORIZED);
        views::Signin {
            form: CredentialsForm {
                email: &submitted_email,
                error: "Email or password is incorrect.",
                ..Default::default()
            },
        }
        .show(req, res);
        return Ok(());
    };
    let pool = depot.get_typed::<PgPool>().unwrap();
    let user: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(pool)
            .await
            .map_err(internal)?;
    let valid = tokio::task::spawn_blocking(move || match user {
        Some((id, hash)) if verify_password(&password, &hash) => Some(id),
        None => {
            hash_password(&password);
            None
        }
        _ => None,
    })
    .await
    .map_err(internal)?;
    if let Some(id) = valid {
        start_session(pool, id, req, depot.get_typed::<Auth>().unwrap(), res).await?;
        redirect(req, res, "/account");
    } else {
        res.status_code(StatusCode::UNAUTHORIZED);
        views::Signin {
            form: CredentialsForm {
                email: &submitted_email,
                error: "Email or password is incorrect.",
                ..Default::default()
            },
        }
        .show(req, res);
    }
    Ok(())
}

#[handler]
async fn signout(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    if let Some(cookie) = req.cookie(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash(cookie.value()))
            .execute(depot.get_typed::<PgPool>().unwrap())
            .await
            .map_err(internal)?;
    }
    let mut cookie = depot.get_typed::<Auth>().unwrap().cookie(String::new());
    cookie.make_removal();
    res.add_cookie(cookie);
    redirect(req, res, "/auth/signin");
    Ok(())
}

async fn show_verification(
    req: &Request,
    depot: &Depot,
    res: &mut Response,
    user: &CurrentUser,
) -> Result<(), StatusError> {
    if user.verified_school().is_some() {
        redirect(req, res, "/account");
        return Ok(());
    }
    if schools::from_email(&user.email).is_none() {
        views::Unsupported.show(req, res);
        return Ok(());
    }
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM email_verifications WHERE user_id = $1 AND expires_at > now())")
        .bind(user.id).fetch_one(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)?;
    views::Verification {
        email: &user.email,
        pending,
        error: "",
    }
    .show(req, res);
    Ok(())
}

#[handler]
async fn verification_page(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    if let Some(user) = current_user(req, depot).await? {
        show_verification(req, depot, res, &user).await?;
    } else {
        redirect(req, res, "/auth/signin");
    }
    Ok(())
}

async fn send_verification(
    pool: &PgPool,
    auth: &Auth,
    user_id: i64,
    email: &str,
    school: &schools::School,
) -> Result<(), StatusError> {
    let token = new_token();
    let issued: Option<i64> = sqlx::query_scalar(
        "INSERT INTO email_verifications (user_id, token_hash, expires_at)
         SELECT id, $2, now() + interval '30 minutes' FROM users
         WHERE id = $1 AND email = $3 AND email_verified_at IS NULL
         ON CONFLICT (user_id) DO UPDATE SET token_hash = EXCLUDED.token_hash,
         expires_at = EXCLUDED.expires_at, sent_at = now()
         WHERE email_verifications.sent_at < now() - interval '1 minute' RETURNING user_id",
    )
    .bind(user_id)
    .bind(token_hash(&token))
    .bind(email)
    .fetch_optional(pool)
    .await
    .map_err(internal)?;
    if issued.is_none() {
        return Ok(());
    }
    let link = format!("{}/auth/verify?token={token}", auth.origin);
    if let Err(error) = auth.mailer.verify_email(email, school.name, &link).await {
        sqlx::query("DELETE FROM email_verifications WHERE token_hash = $1")
            .bind(token_hash(&token))
            .execute(pool)
            .await
            .map_err(internal)?;
        return Err(internal(error));
    }
    Ok(())
}

#[handler]
async fn resend_verification(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let Some(user) = current_user(req, depot).await? else {
        redirect(req, res, "/auth/signin");
        return Ok(());
    };
    if let Some(school) = schools::from_email(&user.email)
        && !user.email_verified
        && send_verification(
            depot.get_typed::<PgPool>().unwrap(),
            depot.get_typed::<Auth>().unwrap(),
            user.id,
            &user.email,
            school,
        )
        .await
        .is_err()
    {
        res.status_code(StatusCode::SERVICE_UNAVAILABLE);
        views::Verification {
            email: &user.email,
            pending: false,
            error: "Couldn't send the email. Please try resending it.",
        }
        .show(req, res);
        return Ok(());
    }
    show_verification(req, depot, res, &user).await
}

#[handler]
async fn verification_status(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let Some(user) = current_user(req, depot).await? else {
        redirect(req, res, "/auth/signin");
        return Ok(());
    };
    if user.verified_school().is_some() {
        redirect(req, res, "/account");
    } else {
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM email_verifications WHERE user_id = $1 AND expires_at > now())")
            .bind(user.id).fetch_one(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)?;
        if pending && is_htmx(req) && schools::from_email(&user.email).is_some() {
            res.status_code(StatusCode::NO_CONTENT);
        } else {
            show_verification(req, depot, res, &user).await?;
        }
    }
    Ok(())
}

#[handler]
async fn verification_form(req: &mut Request, res: &mut Response) {
    let token = req.query::<String>("token").unwrap_or_default();
    views::Verify { token: &token }.show(req, res);
}

#[handler]
async fn verify_email(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let token = req.form::<String>("token").await.unwrap_or_default();
    let pool = depot.get_typed::<PgPool>().unwrap();
    let mut transaction = pool.begin().await.map_err(internal)?;
    let pending: Option<(i64, String)> = sqlx::query_as(
        "DELETE FROM email_verifications AS verification USING users
         WHERE verification.user_id = users.id AND verification.token_hash = $1
         AND verification.expires_at > now() RETURNING users.id, users.email",
    )
    .bind(token_hash(&token))
    .fetch_optional(&mut *transaction)
    .await
    .map_err(internal)?;
    if let Some((id, email)) = pending
        && schools::from_email(&email).is_some()
    {
        let result = sqlx::query(
            "UPDATE users SET email_verified_at = now()
             WHERE id = $1 AND email = $2 AND email_verified_at IS NULL",
        )
        .bind(id)
        .bind(email)
        .execute(&mut *transaction)
        .await
        .map_err(internal)?;
        if result.rows_affected() == 1 {
            transaction.commit().await.map_err(internal)?;
            views::VerificationResult { success: true }.show(req, res);
            return Ok(());
        }
    }
    transaction.commit().await.map_err(internal)?;
    res.status_code(StatusCode::BAD_REQUEST);
    views::VerificationResult { success: false }.show(req, res);
    Ok(())
}

#[cfg(test)]
mod tests;
