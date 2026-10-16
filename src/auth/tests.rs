use super::{Auth, SESSION_COOKIE, routes, admin_routes, hash_password, verify_password, new_token, token_hash, normalize_email};
use salvo::prelude::*;
use salvo::http::cookie::{SameSite, time::Duration};
use sqlx::PgPool;
use salvo::test::{ResponseExt, TestClient};
use std::sync::{Arc, Mutex};

fn test_auth(origin: &str) -> Auth {
    let mut mailer = crate::mail::Mailer::from_env();
    mailer.outbox = Some(Default::default());
    Auth { origin: origin.into(), mailer }
}

fn app(pool: PgPool) -> (Service, Arc<Mutex<Vec<String>>>) {
    let auth = test_auth("http://localhost:8000");
    let outbox = auth.mailer.outbox.clone().unwrap();
    let service = Service::new(Router::new()
        .hoop(affix_state::inject(pool))
        .hoop(affix_state::inject(auth))
        .push(routes()).push(admin_routes()));
    (service, outbox)
}

async fn post(app: &Service, path: &str, cookie: Option<&str>, form: &[(&str, &str)]) -> Response {
    let mut request = TestClient::post(format!("http://localhost:8000{path}"))
        .add_header("origin", "http://localhost:8000", true)
        .add_header("hx-request", "true", true).form(&form);
    if let Some(cookie) = cookie { request = request.add_header("cookie", cookie, true); }
    request.send(app).await
}

async fn get(app: &Service, path: &str, cookie: Option<&str>) -> Response {
    let mut request = TestClient::get(format!("http://localhost:8000{path}"))
        .add_header("hx-request", "true", true);
    if let Some(cookie) = cookie { request = request.add_header("cookie", cookie, true); }
    request.send(app).await
}

fn cookie(response: &Response) -> String {
    format!("{SESSION_COOKIE}={}", response.cookie(SESSION_COOKIE).unwrap().value())
}

fn mailed_token(outbox: &Mutex<Vec<String>>) -> String {
    let messages = outbox.lock().unwrap();
    let message = messages.last().unwrap();
    let (_, body) = message.split_once("\r\n\r\n").unwrap();
    let body = String::from_utf8(quoted_printable::decode(body, quoted_printable::ParseMode::Robust).unwrap()).unwrap();
    body.split("/auth/verify?token=").nth(1).unwrap().split_whitespace().next().unwrap().to_owned()
}

#[test]
fn argon2id_passwords_and_tokens() {
    let hash = hash_password("a long test password");
    assert!(hash.starts_with("$argon2id$v=19$"));
    assert!(verify_password("a long test password", &hash));
    assert!(!verify_password("wrong password", &hash));
    assert!(!verify_password("anything", "invalid hash"));
    assert_ne!(hash, hash_password("a long test password"));
    let token = new_token();
    assert_eq!(hex::decode(&token).unwrap().len(), 32);
    assert_ne!(token, new_token());
    assert_ne!(token_hash(&token), token);
}

#[test]
fn cookies_and_personal_email_validation() {
    for (origin, secure) in [("http://localhost:8000", false), ("https://bound.test", true)] {
        let cookie = test_auth(origin).cookie("token".into());
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.secure(), Some(secure));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.max_age(), Some(Duration::days(30)));
    }
    assert_eq!(normalize_email(" Student@Example.com "), Some("student@example.com".into()));
    for email in ["", "invalid", "a@@example.com", "@example.com"] {
        assert!(normalize_email(email).is_none());
    }
}

#[tokio::test]
async fn forms_validation_and_csrf() {
    let service = Service::new(Router::new()
        .hoop(affix_state::inject(test_auth("http://localhost:8000"))).push(routes()));
    for path in ["signup", "signin"] {
        let mut response = TestClient::get(format!("http://localhost:8000/auth/{path}")).send(&service).await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let html = response.take_string().await.unwrap();
        assert!(html.contains("<!doctype html>"));
        assert!(!html.contains("Duke email"));
    }
    for path in ["signup", "signin", "school", "verify", "resend", "signout"] {
        for origin in [None, Some("https://other.test")] {
            let mut request = TestClient::post(format!("http://localhost:8000/auth/{path}"));
            if let Some(origin) = origin { request = request.add_header("origin", origin, true); }
            assert_eq!(request.send(&service).await.status_code, Some(StatusCode::FORBIDDEN));
        }
    }
    let mut invalid = post(&service, "/auth/signup", None, &[("email", "invalid"), ("password", "long enough password")]).await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
    let html = invalid.take_string().await.unwrap();
    assert!(html.contains("id=\"auth-panel\""));
    assert!(!html.contains("<!doctype"));
    let short = post(&service, "/auth/signup", None, &[("email", "a@example.com"), ("password", "short")]).await;
    assert_eq!(short.status_code, Some(StatusCode::BAD_REQUEST));
    let guest = get(&service, "/auth/school", None).await;
    assert_eq!(guest.headers()["hx-redirect"], "/auth/signin");
}

#[sqlx::test]
async fn signup_school_verification_polling_and_sessions(pool: PgPool) {
    let (service, outbox) = app(pool.clone());
    let credentials = [("email", "Personal@example.com"), ("password", "a long test password"), ("is_admin", "true")];
    let signup = post(&service, "/auth/signup", None, &credentials).await;
    assert_eq!(signup.headers()["hx-redirect"], "/auth/school");
    let first_cookie = cookie(&signup);
    assert!(outbox.lock().unwrap().is_empty(), "Sign-up must not send to the personal email");
    let state: (String, bool, bool) = sqlx::query_as("SELECT email, is_admin, school_email_verified_at IS NOT NULL FROM users")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("personal@example.com".into(), false, false));
    let blocked = get(&service, "/auth/account", Some(&first_cookie)).await;
    assert_eq!(blocked.headers()["hx-redirect"], "/auth/school");
    let unsupported = post(&service, "/auth/school", Some(&first_cookie), &[("school_email", "student@duke.edu.evil.test")]).await;
    assert_eq!(unsupported.status_code, Some(StatusCode::BAD_REQUEST));
    let mut pending = post(&service, "/auth/school", Some(&first_cookie), &[("school_email", "Student@DUKE.edu"), ("school", "fake")]).await;
    assert_eq!(pending.status_code, Some(StatusCode::OK));
    let html = pending.take_string().await.unwrap();
    assert!(html.contains("hx-trigger=\"every 2s\""));
    assert!(html.contains("student@duke.edu"));
    assert!(outbox.lock().unwrap()[0].contains("To: student@duke.edu"));
    let token = mailed_token(&outbox);
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM email_verifications").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, token_hash(&token));
    let waiting = get(&service, "/auth/school/status", Some(&first_cookie)).await;
    assert_eq!(waiting.status_code, Some(StatusCode::NO_CONTENT));
    let guest_poll = get(&service, "/auth/school/status?user_id=1", None).await;
    assert_eq!(guest_poll.headers()["hx-redirect"], "/auth/signin");
    let other_signup = post(&service, "/auth/signup", None, &[("email", "other@example.com"), ("password", "a long test password")]).await;
    let other_cookie = cookie(&other_signup);

    let mut link = TestClient::get(format!("http://localhost:8000/auth/verify?token={token}")).send(&service).await;
    assert!(link.take_string().await.unwrap().contains("hx-trigger=\"load, submit\""));
    let verified: bool = sqlx::query_scalar("SELECT school_email_verified_at IS NOT NULL FROM users WHERE email = 'personal@example.com'")
        .fetch_one(&pool).await.unwrap();
    assert!(!verified, "GET only renders the automatic HTMX confirmation");
    let mut confirmation = post(&service, "/auth/verify", None, &[("token", &token)]).await;
    assert_eq!(confirmation.status_code, Some(StatusCode::OK));
    assert!(confirmation.cookie(SESSION_COOKIE).is_none(), "Verification must not sign in the email tab");
    assert!(confirmation.take_string().await.unwrap().contains("You can now close this page"));
    let state: (String, String, String) = sqlx::query_as(
        "SELECT email, school_email, school FROM users WHERE email = 'personal@example.com'").fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("personal@example.com".into(), "student@duke.edu".into(), "duke".into()));
    let replay = post(&service, "/auth/verify", None, &[("token", &token)]).await;
    assert_eq!(replay.status_code, Some(StatusCode::BAD_REQUEST));
    let ready = get(&service, "/auth/school/status", Some(&first_cookie)).await;
    assert_eq!(ready.headers()["hx-redirect"], "/auth/account");
    let other = get(&service, "/auth/school/status", Some(&other_cookie)).await;
    assert!(other.headers().get("hx-redirect").is_none(), "Polling is scoped to the requesting session");

    let duplicate = post(&service, "/auth/signup", None, &credentials).await;
    assert_eq!(duplicate.status_code, Some(StatusCode::CONFLICT));
    let wrong = post(&service, "/auth/signin", None, &[("email", "personal@example.com"), ("password", "wrong password")]).await;
    assert_eq!(wrong.status_code, Some(StatusCode::UNAUTHORIZED));
    let signin = post(&service, "/auth/signin", Some(&first_cookie), &credentials).await;
    assert_eq!(signin.headers()["hx-redirect"], "/auth/account");
    let second_cookie = cookie(&signin);
    assert_ne!(first_cookie, second_cookie);
    assert_eq!(get(&service, "/auth/account", Some(&first_cookie)).await.headers()["hx-redirect"], "/auth/signin");
    assert_eq!(get(&service, "/auth/account", Some(&second_cookie)).await.status_code, Some(StatusCode::OK));
    let logout = post(&service, "/auth/signout", Some(&second_cookie), &[]).await;
    assert_eq!(logout.cookie(SESSION_COOKIE).unwrap().max_age(), Some(Duration::ZERO));
    assert_eq!(get(&service, "/auth/account", Some(&second_cookie)).await.headers()["hx-redirect"], "/auth/signin");
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(get(&service, "/auth/school/status", Some(&other_cookie)).await.headers()["hx-redirect"], "/auth/signin");
}

#[sqlx::test]
async fn resends_expiry_and_school_email_uniqueness(pool: PgPool) {
    let (service, outbox) = app(pool.clone());
    let signup = post(&service, "/auth/signup", None, &[("email", "one@example.com"), ("password", "a long test password")]).await;
    let first = cookie(&signup);
    post(&service, "/auth/school", Some(&first), &[("school_email", "student@duke.edu")]).await;
    let old = mailed_token(&outbox);
    post(&service, "/auth/resend", Some(&first), &[]).await;
    assert_eq!(outbox.lock().unwrap().len(), 1);
    sqlx::query("UPDATE email_verifications SET expires_at = now() - interval '1 second', sent_at = now() - interval '2 minutes'")
        .execute(&pool).await.unwrap();
    let expired = post(&service, "/auth/verify", None, &[("token", &old)]).await;
    assert_eq!(expired.status_code, Some(StatusCode::BAD_REQUEST));
    let mut stopped = get(&service, "/auth/school/status", Some(&first)).await;
    assert!(!stopped.take_string().await.unwrap().contains("every 2s"), "Expired links stop polling");
    post(&service, "/auth/resend", Some(&first), &[]).await;
    let renewed = mailed_token(&outbox);
    assert_ne!(old, renewed);
    assert_eq!(post(&service, "/auth/verify", None, &[("token", &old)]).await.status_code, Some(StatusCode::BAD_REQUEST));

    // Two accounts can request the same address, but only one can link it.
    let signup = post(&service, "/auth/signup", None, &[("email", "two@example.com"), ("password", "a long test password")]).await;
    let second = cookie(&signup);
    post(&service, "/auth/school", Some(&second), &[("school_email", "student@duke.edu")]).await;
    let competing = mailed_token(&outbox);
    assert_eq!(post(&service, "/auth/verify", None, &[("token", &renewed)]).await.status_code, Some(StatusCode::OK));
    assert_eq!(post(&service, "/auth/verify", None, &[("token", &competing)]).await.status_code, Some(StatusCode::CONFLICT));
    let response = post(&service, "/auth/school", Some(&second), &[("school_email", "student@duke.edu")]).await;
    assert_eq!(response.status_code, Some(StatusCode::CONFLICT));
    sqlx::query("UPDATE email_verifications SET sent_at = now() - interval '2 minutes'").execute(&pool).await.unwrap();
    post(&service, "/auth/school", Some(&second), &[("school_email", "another@duke.edu")]).await;
    let replacement = mailed_token(&outbox);
    assert_eq!(post(&service, "/auth/verify", None, &[("token", &competing)]).await.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(post(&service, "/auth/verify", None, &[("token", &replacement)]).await.status_code, Some(StatusCode::OK));
}

#[sqlx::test]
async fn admin_permissions_require_verified_school_and_follow_role_changes(pool: PgPool) {
    let (service, _) = app(pool.clone());
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ('personal@example.com', 'unused') RETURNING id")
        .fetch_one(&pool).await.unwrap();
    let token = new_token();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(token_hash(&token)).bind(user_id).execute(&pool).await.unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");
    assert_eq!(get(&service, "/admin", None).await.headers()["hx-redirect"], "/auth/signin");
    assert_eq!(get(&service, "/admin", Some(&cookie)).await.headers()["hx-redirect"], "/auth/school");
    sqlx::query("UPDATE users SET school_email = 'student@duke.edu', school = 'duke', school_email_verified_at = now()")
        .execute(&pool).await.unwrap();
    for (is_admin, expected) in [(false, StatusCode::FORBIDDEN), (true, StatusCode::OK), (false, StatusCode::FORBIDDEN)] {
        sqlx::query("UPDATE users SET is_admin = $1").bind(is_admin).execute(&pool).await.unwrap();
        assert_eq!(get(&service, "/admin", Some(&cookie)).await.status_code, Some(expected));
    }
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(get(&service, "/admin", Some(&cookie)).await.headers()["hx-redirect"], "/auth/signin");
}

#[sqlx::test(migrations = false)]
async fn migration_preserves_existing_duke_verification(pool: PgPool) {
    for sql in [include_str!("../../migrations/0001_auth.sql"), include_str!("../../migrations/0002_admin.sql"), include_str!("../../migrations/0003_email_verification.sql")] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO users (email, password_hash, email_verified_at) VALUES ('student@duke.edu', 'unused', now())")
        .execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0004_school_emails.sql")).execute(&pool).await.unwrap();
    let state: (String, String, bool) = sqlx::query_as("SELECT school_email, school, school_email_verified_at IS NOT NULL FROM users")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("student@duke.edu".into(), "duke".into(), true));
}
