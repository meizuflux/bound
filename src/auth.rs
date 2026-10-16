use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use askama::Template;
use salvo::http::Method;
use salvo::http::cookie::{Cookie, SameSite, time::Duration};
use salvo::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::{schools, templates::HtmlTemplate};

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
                .trim_end_matches('/').to_owned(),
            mailer: crate::mail::Mailer::from_env(),
        }
    }

    fn cookie(&self, token: String) -> Cookie<'static> {
        Cookie::build((SESSION_COOKIE, token))
            .http_only(true).same_site(SameSite::Lax)
            .secure(self.origin.starts_with("https://")).path("/")
            .max_age(Duration::days(30)).build()
    }
}

pub fn routes() -> Router {
    Router::with_path("auth").hoop(protect)
        .push(Router::with_path("signup").get(signup_form).post(signup))
        .push(Router::with_path("signin").get(signin_form).post(signin))
        .push(Router::with_path("account").get(account))
        .push(Router::with_path("signout").post(signout))
        .push(Router::with_path("school").get(school_form).post(submit_school))
        .push(Router::with_path("school/status").get(school_status))
        .push(Router::with_path("verify").get(verification_form).post(verify_email))
        .push(Router::with_path("resend").post(resend_verification))
}

pub fn admin_routes() -> Router {
    Router::with_path("admin").hoop(protect).hoop(require_admin).get(admin)
}

#[derive(sqlx::FromRow)]
pub struct CurrentUser {
    pub id: i64,
    pub email: String,
    pub is_admin: bool,
    school_email: Option<String>,
    school: Option<String>,
    school_verified: bool,
}

impl CurrentUser {
    fn verified_school(&self) -> Option<&'static schools::School> {
        let school = schools::from_email(self.school_email.as_deref()?)?;
        (self.school_verified && self.school.as_deref() == Some(school.id)).then_some(school)
    }
}

async fn current_user(req: &Request, depot: &Depot) -> Result<Option<CurrentUser>, StatusError> {
    let Some(cookie) = req.cookie(SESSION_COOKIE) else { return Ok(None); };
    sqlx::query_as(
        "SELECT users.id, users.email, users.is_admin, users.school_email, users.school,
         users.school_email_verified_at IS NOT NULL AS school_verified FROM sessions
         JOIN users ON users.id = sessions.user_id
         WHERE sessions.token_hash = $1 AND sessions.expires_at > now()")
        .bind(token_hash(cookie.value()))
        .fetch_optional(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)
}

/// Apply to admin routers; handlers can read CurrentUser from Depot.
#[handler]
pub async fn require_admin(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    match current_user(req, depot).await {
        Ok(Some(user)) if user.verified_school().is_none() => redirect(req, res, "/auth/school"),
        Ok(Some(user)) if user.is_admin => { depot.insert_typed(user); return; }
        Ok(Some(_)) => { res.render(StatusError::forbidden()); }
        Ok(None) => redirect(req, res, "/auth/signin"),
        Err(error) => { res.render(error); }
    }
    ctrl.skip_rest();
}

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate { user_id: i64, email: String }

#[handler]
async fn admin(depot: &mut Depot) -> HtmlTemplate<AdminTemplate> {
    let user = depot.get_typed::<CurrentUser>().unwrap();
    HtmlTemplate(AdminTemplate { user_id: user.id, email: user.email.clone() })
}

#[derive(Template)]
#[template(path = "auth.html")]
struct AuthTemplate<'a> { title: &'a str, mode: &'a str, message: &'a str, token: &'a str }

#[derive(Template)]
#[template(path = "auth.html", block = "content")]
struct AuthFragment<'a> { title: &'a str, mode: &'a str, message: &'a str, token: &'a str }

fn is_htmx(req: &Request) -> bool {
    req.headers().get("hx-request").is_some_and(|value| value == "true")
}

fn redirect(req: &Request, res: &mut Response, path: &'static str) {
    if is_htmx(req) {
        res.status_code(StatusCode::OK);
        res.headers_mut().insert("hx-redirect", path.parse().unwrap());
    } else {
        res.render(Redirect::other(path));
    }
}

fn page(req: &Request, res: &mut Response, mode: &str, message: &str) {
    render_page(req, res, mode, message, "");
}

fn render_page(req: &Request, res: &mut Response, mode: &str, message: &str, token: &str) {
    let title = match mode {
        "signup" => "Create an account",
        "signin" => "Sign in",
        "school" => "Enter your school email address",
        "pending" => "Check your school inbox",
        "verify" => "Verify your school email",
        "verified" => "School email verified",
        "invalid" => "Unable to verify this email",
        _ => "Your account",
    };
    if is_htmx(req) {
        res.render(HtmlTemplate(AuthFragment { title, mode, message, token }));
    } else {
        res.render(HtmlTemplate(AuthTemplate { title, mode, message, token }));
    }
}

#[handler]
async fn protect(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    res.headers_mut().insert("cache-control", "no-store".parse().unwrap());
    res.headers_mut().insert("vary", "HX-Request".parse().unwrap());
    res.headers_mut().insert("referrer-policy", "no-referrer".parse().unwrap());
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
    page(req, res, "signup", "Use any email address and a password with at least 12 characters.");
}

#[handler]
async fn signin_form(req: &mut Request, res: &mut Response) { page(req, res, "signin", ""); }

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

fn token_hash(token: &str) -> String { hex::encode(Sha256::digest(token.as_bytes())) }

fn password_hasher() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
}

fn hash_password(password: &str) -> String {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("OS randomness unavailable");
    password_hasher().hash_password(password.as_bytes(), &SaltString::encode_b64(&salt).unwrap())
        .expect("Failed to hash password").to_string()
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash|
        password_hasher().verify_password(password.as_bytes(), &hash).is_ok())
}

fn internal(error: impl std::fmt::Display) -> StatusError {
    tracing::error!(%error, "Authentication operation failed");
    StatusError::internal_server_error()
}

async fn start_session(pool: &PgPool, user_id: i64, req: &Request, auth: &Auth, res: &mut Response, target: &'static str)
    -> Result<(), StatusError>
{
    let token = new_token();
    let mut transaction = pool.begin().await.map_err(internal)?;
    if let Some(old) = req.cookie(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash(old.value())).execute(&mut *transaction).await.map_err(internal)?;
    }
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '30 days')")
        .bind(token_hash(&token)).bind(user_id).execute(&mut *transaction).await.map_err(internal)?;
    transaction.commit().await.map_err(internal)?;
    res.add_cookie(auth.cookie(token));
    redirect(req, res, target);
    Ok(())
}

#[handler]
async fn signup(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let email = req.form::<String>("email").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    let Some(email) = normalize_email(&email) else {
        res.status_code(StatusCode::BAD_REQUEST);
        page(req, res, "signup", "Enter a valid email address.");
        return Ok(());
    };
    if !(12..=128).contains(&password.chars().count()) {
        res.status_code(StatusCode::BAD_REQUEST);
        page(req, res, "signup", "Use a password between 12 and 128 characters.");
        return Ok(());
    }
    let hash = tokio::task::spawn_blocking(move || hash_password(&password)).await.map_err(internal)?;
    let pool = depot.get_typed::<PgPool>().unwrap();
    let id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2) ON CONFLICT (email) DO NOTHING RETURNING id")
        .bind(email).bind(hash).fetch_optional(pool).await.map_err(internal)?;
    if let Some(id) = id {
        start_session(pool, id, req, depot.get_typed::<Auth>().unwrap(), res, "/auth/school").await?;
    } else {
        res.status_code(StatusCode::CONFLICT);
        page(req, res, "signup", "Unable to create this account. Try signing in.");
    }
    Ok(())
}

#[handler]
async fn signin(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let email = req.form::<String>("email").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    if password.chars().count() > 128 {
        res.status_code(StatusCode::BAD_REQUEST);
        page(req, res, "signin", "Email or password is incorrect.");
        return Ok(());
    }
    let pool = depot.get_typed::<PgPool>().unwrap();
    let user: Option<(i64, String)> = sqlx::query_as("SELECT id, password_hash FROM users WHERE email = $1")
        .bind(normalize_email(&email).unwrap_or_default()).fetch_optional(pool).await.map_err(internal)?;
    let valid = tokio::task::spawn_blocking(move || match user {
        Some((id, hash)) if verify_password(&password, &hash) => Some(id),
        None => { hash_password(&password); None }
        _ => None,
    }).await.map_err(internal)?;
    if let Some(id) = valid {
        start_session(pool, id, req, depot.get_typed::<Auth>().unwrap(), res, "/auth/account").await?;
    } else {
        res.status_code(StatusCode::UNAUTHORIZED);
        page(req, res, "signin", "Email or password is incorrect.");
    }
    Ok(())
}

#[handler]
async fn account(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    match current_user(req, depot).await? {
        Some(user) if user.verified_school().is_some() => {
            page(req, res, "account", &format!("{} · {}", user.email, user.verified_school().unwrap().name));
        }
        Some(_) => redirect(req, res, "/auth/school"),
        None => redirect(req, res, "/auth/signin"),
    }
    Ok(())
}

#[handler]
async fn signout(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    if let Some(cookie) = req.cookie(SESSION_COOKIE) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash(cookie.value())).execute(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)?;
    }
    let mut cookie = depot.get_typed::<Auth>().unwrap().cookie(String::new());
    cookie.make_removal();
    res.add_cookie(cookie);
    redirect(req, res, "/auth/signin");
    Ok(())
}

async fn show_school(req: &Request, depot: &Depot, res: &mut Response, user: &CurrentUser) -> Result<(), StatusError> {
    if user.verified_school().is_some() {
        redirect(req, res, "/auth/account"); return Ok(());
    }
    let pending: Option<String> = sqlx::query_scalar(
        "SELECT school_email FROM email_verifications WHERE user_id = $1 AND expires_at > now()")
        .bind(user.id).fetch_optional(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)?;
    if let Some(email) = pending.filter(|_| req.query::<String>("edit").is_none()) {
        page(req, res, "pending", &format!("We sent a verification link to {email}. This page will update when you verify."));
    } else {
        page(req, res, "school", &format!("We'll find your school from your email address. Currently supporting {}.", schools::supported_names()));
    }
    Ok(())
}

#[handler]
async fn school_form(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    if let Some(user) = current_user(req, depot).await? {
        show_school(req, depot, res, &user).await?;
    } else {
        redirect(req, res, "/auth/signin");
    }
    Ok(())
}

#[handler]
async fn submit_school(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let Some(user) = current_user(req, depot).await? else {
        redirect(req, res, "/auth/signin"); return Ok(());
    };
    if user.verified_school().is_some() {
        redirect(req, res, "/auth/account"); return Ok(());
    }
    let email = req.form::<String>("school_email").await.unwrap_or_default();
    let email = normalize_email(&email);
    let school = email.as_deref().and_then(schools::from_email);
    let (Some(email), Some(school)) = (email, school) else {
        res.status_code(StatusCode::BAD_REQUEST);
        page(req, res, "school", &format!("That school email isn't supported yet. Currently supporting {}.", schools::supported_names()));
        return Ok(());
    };
    let pool = depot.get_typed::<PgPool>().unwrap();
    let taken: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE school_email = $1 AND id <> $2)")
        .bind(&email).bind(user.id).fetch_one(pool).await.map_err(internal)?;
    if taken {
        res.status_code(StatusCode::CONFLICT);
        page(req, res, "school", "That school email is already linked to an account.");
    } else if send_verification(pool, depot.get_typed::<Auth>().unwrap(), user.id, &email, school).await.is_err() {
        res.status_code(StatusCode::SERVICE_UNAVAILABLE);
        page(req, res, "school", "Couldn't send the email. Please try again.");
    } else {
        show_school(req, depot, res, &user).await?;
    }
    Ok(())
}

async fn send_verification(pool: &PgPool, auth: &Auth, user_id: i64, email: &str, school: &schools::School) -> Result<(), StatusError> {
    let token = new_token();
    let issued: Option<i64> = sqlx::query_scalar(
        "INSERT INTO email_verifications (user_id, token_hash, expires_at, school_email, school)
         SELECT id, $2, now() + interval '30 minutes', $3, $4 FROM users
         WHERE id = $1 AND school_email_verified_at IS NULL
         ON CONFLICT (user_id) DO UPDATE SET token_hash = EXCLUDED.token_hash,
         expires_at = EXCLUDED.expires_at, school_email = EXCLUDED.school_email,
         school = EXCLUDED.school, sent_at = now()
         WHERE email_verifications.sent_at < now() - interval '1 minute' RETURNING user_id")
        .bind(user_id).bind(token_hash(&token)).bind(email).bind(school.id)
        .fetch_optional(pool).await.map_err(internal)?;
    if issued.is_none() { return Ok(()); }
    let link = format!("{}/auth/verify?token={token}", auth.origin);
    if let Err(error) = auth.mailer.verify_email(email, school.name, &link).await {
        sqlx::query("DELETE FROM email_verifications WHERE token_hash = $1")
            .bind(token_hash(&token)).execute(pool).await.map_err(internal)?;
        return Err(internal(error));
    }
    Ok(())
}

#[handler]
async fn resend_verification(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let Some(user) = current_user(req, depot).await? else {
        redirect(req, res, "/auth/signin"); return Ok(());
    };
    let pool = depot.get_typed::<PgPool>().unwrap();
    let email: Option<String> = sqlx::query_scalar("SELECT school_email FROM email_verifications WHERE user_id = $1")
        .bind(user.id).fetch_optional(pool).await.map_err(internal)?;
    if let Some(email) = email {
        if let Some(school) = schools::from_email(&email) {
            if send_verification(pool, depot.get_typed::<Auth>().unwrap(), user.id, &email, school).await.is_err() {
                res.status_code(StatusCode::SERVICE_UNAVAILABLE);
                page(req, res, "school", "Couldn't send the email. Please try again.");
                return Ok(());
            }
        }
    }
    show_school(req, depot, res, &user).await
}

#[handler]
async fn school_status(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let Some(user) = current_user(req, depot).await? else {
        redirect(req, res, "/auth/signin"); return Ok(());
    };
    if user.verified_school().is_some() {
        redirect(req, res, "/auth/account");
    } else {
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM email_verifications WHERE user_id = $1 AND expires_at > now())")
            .bind(user.id).fetch_one(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)?;
        if pending && is_htmx(req) {
            res.status_code(StatusCode::NO_CONTENT);
        } else {
            show_school(req, depot, res, &user).await?;
        }
    }
    Ok(())
}

#[handler]
async fn verification_form(req: &mut Request, res: &mut Response) {
    let token = req.query::<String>("token").unwrap_or_default();
    render_page(req, res, "verify", "Verifying your school email…", &token);
}

#[handler]
async fn verify_email(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let token = req.form::<String>("token").await.unwrap_or_default();
    let pool = depot.get_typed::<PgPool>().unwrap();
    let mut transaction = pool.begin().await.map_err(internal)?;
    let pending: Option<(i64, String, String)> = sqlx::query_as(
        "DELETE FROM email_verifications WHERE token_hash = $1 AND expires_at > now() RETURNING user_id, school_email, school")
        .bind(token_hash(&token)).fetch_optional(&mut *transaction).await.map_err(internal)?;
    if let Some((id, email, school_id)) = pending {
        if schools::from_email(&email).is_some_and(|school| school.id == school_id) {
            let result = sqlx::query(
                "UPDATE users SET school_email = $2, school = $3, school_email_verified_at = now()
                 WHERE id = $1 AND school_email_verified_at IS NULL")
                .bind(id).bind(email).bind(school_id).execute(&mut *transaction).await;
            match result {
                Ok(result) if result.rows_affected() == 1 => {
                    transaction.commit().await.map_err(internal)?;
                    page(req, res, "verified", "Your school email is verified. You can now close this page and return to Bound.");
                    return Ok(());
                }
                Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                    transaction.rollback().await.map_err(internal)?;
                    res.status_code(StatusCode::CONFLICT);
                    page(req, res, "invalid", "That school email is already linked to another account.");
                    return Ok(());
                }
                Err(error) => return Err(internal(error)),
                _ => {}
            }
        }
    }
    transaction.commit().await.map_err(internal)?;
    res.status_code(StatusCode::BAD_REQUEST);
    page(req, res, "invalid", "This link is invalid, expired, or already used. Return to Bound to request a new one.");
    Ok(())
}

#[cfg(test)]
mod tests;
