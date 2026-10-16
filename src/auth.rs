use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use argon2::password_hash::SaltString;
use askama::Template;
use salvo::http::cookie::{Cookie, SameSite, time::Duration};
use salvo::http::Method;
use salvo::prelude::*;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::templates::HtmlTemplate;

const SESSION_COOKIE: &str = "bound_session";

#[derive(Clone)]
pub struct Auth {
    origin: String,
}

impl Auth {
    pub fn from_env() -> Self {
        Self {
            origin: std::env::var("APP_URL")
                .unwrap_or_else(|_| "http://localhost:8000".into())
                .trim_end_matches('/').to_owned(),
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
    Router::with_path("auth").hoop(protect)
        .push(Router::with_path("signup").get(signup_form).post(signup))
        .push(Router::with_path("signin").get(signin_form).post(signin))
        .push(Router::with_path("account").get(account))
        .push(Router::with_path("signout").post(signout))
}

pub fn admin_routes() -> Router {
    Router::with_path("admin").hoop(protect).hoop(require_admin).get(admin)
}

#[derive(sqlx::FromRow)]
pub struct CurrentUser {
    pub id: i64,
    pub email: String,
    pub is_admin: bool,
}

async fn current_user(req: &Request, depot: &Depot) -> Result<Option<CurrentUser>, StatusError> {
    let Some(cookie) = req.cookie(SESSION_COOKIE) else { return Ok(None); };
    sqlx::query_as(
        "SELECT users.id, users.email, users.is_admin FROM sessions \
         JOIN users ON users.id = sessions.user_id \
         WHERE sessions.token_hash = $1 AND sessions.expires_at > now()")
        .bind(token_hash(cookie.value()))
        .fetch_optional(depot.get_typed::<PgPool>().unwrap())
        .await.map_err(internal)
}

/// Apply to an admin router and its children; handlers can read CurrentUser from Depot.
#[handler]
pub async fn require_admin(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    match current_user(req, depot).await {
        Ok(Some(user)) if user.is_admin => {
            depot.insert_typed(user);
            return;
        }
        Ok(Some(_)) => { res.render(StatusError::forbidden()); }
        Ok(None) => { res.render(Redirect::other("/auth/signin")); }
        Err(error) => { res.render(error); }
    }
    ctrl.skip_rest();
}

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    user_id: i64,
    email: String,
}

#[handler]
async fn admin(depot: &mut Depot) -> HtmlTemplate<AdminTemplate> {
    let user = depot.get_typed::<CurrentUser>().unwrap();
    HtmlTemplate(AdminTemplate { user_id: user.id, email: user.email.clone() })
}

#[derive(Template)]
#[template(path = "auth.html")]
struct AuthTemplate<'a> {
    title: &'a str,
    mode: &'a str,
    message: &'a str,
}

fn page(res: &mut Response, mode: &str, message: &str) {
    let title = match mode {
        "signup" => "Create an account",
        "signin" => "Sign in",
        _ => "Your account",
    };
    res.render(HtmlTemplate(AuthTemplate { title, mode, message }));
}

// Restrict form submissions to our origin, including sign-in and sign-out.
#[handler]
async fn protect(req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
    res.headers_mut().insert("cache-control", "no-store".parse().unwrap());
    if req.method() == Method::POST {
        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
        if origin != Some(depot.get_typed::<Auth>().unwrap().origin.as_str()) {
            res.render(StatusError::forbidden());
            ctrl.skip_rest();
        }
    }
}

#[handler]
async fn signup_form(res: &mut Response) { page(res, "signup", "Use at least 12 characters for your password."); }

#[handler]
async fn signin_form(res: &mut Response) { page(res, "signin", ""); }

fn normalize_email(email: &str) -> Option<String> {
    let email = email.trim().to_lowercase();
    let (local, domain) = email.split_once('@')?;
    (!local.is_empty() && !domain.is_empty() && !domain.contains('@')
        && email.len() <= 254 && !email.chars().any(char::is_whitespace))
        .then_some(email)
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

async fn start_session(pool: &PgPool, user_id: i64, req: &Request, auth: &Auth, res: &mut Response)
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
    res.render(Redirect::other("/auth/account"));
    Ok(())
}

#[handler]
async fn signup(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let email = req.form::<String>("email").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    let Some(email) = normalize_email(&email) else {
        res.status_code(StatusCode::BAD_REQUEST);
        page(res, "signup", "Enter a valid email address.");
        return Ok(());
    };
    if !(12..=128).contains(&password.chars().count()) {
        res.status_code(StatusCode::BAD_REQUEST);
        page(res, "signup", "Use a password between 12 and 128 characters.");
        return Ok(());
    }
    let hash = tokio::task::spawn_blocking(move || hash_password(&password)).await.map_err(internal)?;
    let pool = depot.get_typed::<PgPool>().unwrap();
    let user_id: Option<i64> = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2) ON CONFLICT (email) DO NOTHING RETURNING id")
        .bind(email).bind(hash).fetch_optional(pool).await.map_err(internal)?;
    match user_id {
        Some(id) => start_session(pool, id, req, depot.get_typed::<Auth>().unwrap(), res).await?,
        None => {
            res.status_code(StatusCode::CONFLICT);
            page(res, "signup", "Unable to create this account. Try signing in.");
        }
    }
    Ok(())
}

#[handler]
async fn signin(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let email = req.form::<String>("email").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    if password.chars().count() > 128 {
        res.status_code(StatusCode::BAD_REQUEST);
        page(res, "signin", "Email or password is incorrect.");
        return Ok(());
    }
    let pool = depot.get_typed::<PgPool>().unwrap();
    let user: Option<(i64, String)> = sqlx::query_as("SELECT id, password_hash FROM users WHERE email = $1")
        .bind(normalize_email(&email).unwrap_or_default()).fetch_optional(pool).await.map_err(internal)?;
    // Missing accounts still perform password hashing to avoid a quick timing signal.
    let valid = tokio::task::spawn_blocking(move || {
        match user {
            Some((id, hash)) if verify_password(&password, &hash) => Some(id),
            None => { hash_password(&password); None }
            _ => None,
        }
    }).await.map_err(internal)?;
    if let Some(id) = valid {
        start_session(pool, id, req, depot.get_typed::<Auth>().unwrap(), res).await?;
    } else {
        res.status_code(StatusCode::UNAUTHORIZED);
        page(res, "signin", "Email or password is incorrect.");
    }
    Ok(())
}

#[handler]
async fn account(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    match current_user(req, depot).await? {
        Some(user) => page(res, "account", &user.email),
        None => { res.render(Redirect::other("/auth/signin")); }
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
    res.render(Redirect::other("/auth/signin"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use salvo::test::TestClient;

    fn service() -> Service {
        Service::new(Router::new()
            .hoop(affix_state::inject(Auth { origin: "http://localhost:8000".into() }))
            .push(routes()))
    }

    #[tokio::test]
    async fn forms_render_and_private_account_redirects() {
        for path in ["signup", "signin"] {
            let response = TestClient::get(format!("http://localhost:8000/auth/{path}"))
                .send(&service()).await;
            assert_eq!(response.status_code, Some(StatusCode::OK));
            assert_eq!(response.headers()["cache-control"], "no-store");
        }
        let response = TestClient::get("http://localhost:8000/auth/account").send(&service()).await;
        assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
        assert_eq!(response.headers()["location"], "/auth/signin");
    }

    #[tokio::test]
    async fn cross_origin_and_missing_origin_posts_are_rejected() {
        for path in ["signup", "signin", "signout"] {
            for origin in [None, Some("https://other.test")] {
                let mut request = TestClient::post(format!("http://localhost:8000/auth/{path}"));
                if let Some(origin) = origin { request = request.add_header("origin", origin, true); }
                let response = request.send(&service()).await;
                assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
            }
        }
    }

    #[tokio::test]
    async fn signup_rejects_invalid_credentials_before_database_access() {
        for (email, password) in [("invalid", "long enough password"), ("a@example.com", "short")] {
            let response = TestClient::post("http://localhost:8000/auth/signup")
                .add_header("origin", "http://localhost:8000", true)
                .form(&[("email", email), ("password", password)])
                .send(&service()).await;
            assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
        }
    }

    #[test]
    fn passwords_use_salted_argon2id_and_reject_wrong_passwords() {
        let hash = hash_password("a long test password");
        assert!(hash.starts_with("$argon2id$v=19$"));
        assert!(verify_password("a long test password", &hash));
        assert!(!verify_password("wrong password", &hash));
        assert!(!verify_password("anything", "invalid hash"));
        assert_ne!(hash, hash_password("a long test password"));
    }

    #[sqlx::test]
    async fn session_lifecycle(pool: PgPool) {
        let app = Service::new(Router::new()
            .hoop(affix_state::inject(pool.clone()))
            .hoop(affix_state::inject(Auth { origin: "http://localhost:8000".into() }))
            .push(routes()));
        let credentials = [("email", "Student@example.com"), ("password", "a long test password")];
        let response = TestClient::post("http://localhost:8000/auth/signup")
            .add_header("origin", "http://localhost:8000", true)
            .form(&credentials).send(&app).await;
        assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
        let cookie = response.cookie(SESSION_COOKIE).unwrap();
        let first_cookie = format!("{SESSION_COOKIE}={}", cookie.value());
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM sessions")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(stored, token_hash(cookie.value()));
        assert_ne!(stored, cookie.value());

        let duplicate = TestClient::post("http://localhost:8000/auth/signup")
            .add_header("origin", "http://localhost:8000", true)
            .form(&credentials).send(&app).await;
        assert_eq!(duplicate.status_code, Some(StatusCode::CONFLICT));
        let wrong = TestClient::post("http://localhost:8000/auth/signin")
            .add_header("origin", "http://localhost:8000", true)
            .form(&[("email", "student@example.com"), ("password", "the wrong password")])
            .send(&app).await;
        assert_eq!(wrong.status_code, Some(StatusCode::UNAUTHORIZED));
        assert!(wrong.cookie(SESSION_COOKIE).is_none());

        let response = TestClient::post("http://localhost:8000/auth/signin")
            .add_header("origin", "http://localhost:8000", true)
            .add_header("cookie", &first_cookie, true)
            .form(&credentials).send(&app).await;
        assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
        let second_cookie = format!("{SESSION_COOKIE}={}", response.cookie(SESSION_COOKIE).unwrap().value());
        assert_ne!(first_cookie, second_cookie);
        let old = TestClient::get("http://localhost:8000/auth/account")
            .add_header("cookie", &first_cookie, true).send(&app).await;
        assert_eq!(old.status_code, Some(StatusCode::SEE_OTHER));
        let account_response = TestClient::get("http://localhost:8000/auth/account")
            .add_header("cookie", &second_cookie, true).send(&app).await;
        assert_eq!(account_response.status_code, Some(StatusCode::OK));

        let logout = TestClient::post("http://localhost:8000/auth/signout")
            .add_header("origin", "http://localhost:8000", true)
            .add_header("cookie", &second_cookie, true).send(&app).await;
        assert_eq!(logout.status_code, Some(StatusCode::SEE_OTHER));
        assert_eq!(logout.cookie(SESSION_COOKIE).unwrap().max_age(), Some(Duration::ZERO));
        let replay = TestClient::get("http://localhost:8000/auth/account")
            .add_header("cookie", &second_cookie, true).send(&app).await;
        assert_eq!(replay.status_code, Some(StatusCode::SEE_OTHER));

        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users").fetch_one(&pool).await.unwrap();
        let expired = new_token();
        sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() - interval '1 second')")
            .bind(token_hash(&expired)).bind(user_id).execute(&pool).await.unwrap();
        let response = TestClient::get("http://localhost:8000/auth/account")
            .add_header("cookie", format!("{SESSION_COOKIE}={expired}"), true).send(&app).await;
        assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
    }

    #[test]
    fn tokens_are_random_and_stored_as_hashes() {
        let token = new_token();
        assert_eq!(hex::decode(&token).unwrap().len(), 32);
        assert_ne!(token, new_token());
        assert_ne!(token_hash(&token), token);
        assert_eq!(token_hash(&token), token_hash(&token));
    }

    #[sqlx::test]
    async fn admin_permissions_follow_database_changes_and_session_expiry(pool: PgPool) {
        let app = Service::new(Router::new()
            .hoop(affix_state::inject(pool.clone()))
            .hoop(affix_state::inject(Auth { origin: "http://localhost:8000".into() }))
            .push(admin_routes()));
        let user_id: i64 = sqlx::query_scalar(
            "INSERT INTO users (email, password_hash) VALUES ('user@example.com', 'unused') RETURNING id")
            .fetch_one(&pool).await.unwrap();
        let is_admin: bool = sqlx::query_scalar("SELECT is_admin FROM users WHERE id = $1")
            .bind(user_id).fetch_one(&pool).await.unwrap();
        assert!(!is_admin);
        let token = new_token();
        sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')")
            .bind(token_hash(&token)).bind(user_id).execute(&pool).await.unwrap();
        let cookie = format!("{SESSION_COOKIE}={token}");

        let guest = TestClient::get("http://localhost:8000/admin").send(&app).await;
        assert_eq!(guest.status_code, Some(StatusCode::SEE_OTHER));
        assert_eq!(guest.headers()["location"], "/auth/signin");
        let forged = TestClient::get("http://localhost:8000/admin")
            .add_header("cookie", "bound_session=forged; is_admin=true", true).send(&app).await;
        assert_eq!(forged.status_code, Some(StatusCode::SEE_OTHER));

        // The same session sees grants and revocations without signing in again.
        for (is_admin, expected) in [
            (false, StatusCode::FORBIDDEN),
            (true, StatusCode::OK),
            (false, StatusCode::FORBIDDEN),
        ] {
            sqlx::query("UPDATE users SET is_admin = $1 WHERE id = $2")
                .bind(is_admin).bind(user_id).execute(&pool).await.unwrap();
            let response = TestClient::get("http://localhost:8000/admin")
                .add_header("cookie", &cookie, true).send(&app).await;
            assert_eq!(response.status_code, Some(expected));
            assert_eq!(response.headers()["cache-control"], "no-store");
        }

        sqlx::query("UPDATE users SET is_admin = true WHERE id = $1")
            .bind(user_id).execute(&pool).await.unwrap();
        sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'")
            .execute(&pool).await.unwrap();
        let expired = TestClient::get("http://localhost:8000/admin")
            .add_header("cookie", &cookie, true).send(&app).await;
        assert_eq!(expired.status_code, Some(StatusCode::SEE_OTHER));
    }

    #[test]
    fn cookies_are_scoped_and_secure_on_https() {
        for (origin, secure) in [("http://localhost:8000", false), ("https://bound.test", true)] {
            let cookie = Auth { origin: origin.into() }.cookie("token".into());
            assert_eq!(cookie.http_only(), Some(true));
            assert_eq!(cookie.same_site(), Some(SameSite::Lax));
            assert_eq!(cookie.secure(), Some(secure));
            assert_eq!(cookie.path(), Some("/"));
            assert_eq!(cookie.max_age(), Some(Duration::days(30)));
        }
    }

    #[test]
    fn email_validation_does_not_restrict_university_domains() {
        assert_eq!(normalize_email(" Student@Example.com "), Some("student@example.com".into()));
        assert_eq!(normalize_email("student@college.edu"), Some("student@college.edu".into()));
        for email in ["", "missing-at", "@example.com", "a@", "a@@example.com", "a b@example.com"] {
            assert!(normalize_email(email).is_none());
        }
    }
}
