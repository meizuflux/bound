use super::{
    Auth, SESSION_COOKIE, admin_routes, hash_password, new_token, normalize_email, routes,
    token_hash, verify_password,
};
use salvo::http::cookie::{SameSite, time::Duration};
use salvo::prelude::*;
use salvo::test::{ResponseExt, TestClient};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

const PASSWORD: &str = "a long test password";
const SINGLE_EMAIL_MIGRATION: &str = include_str!("../../migrations/0005_single_school_email.sql");

fn test_auth(origin: &str) -> Auth {
    let mut mailer = crate::mail::Mailer::from_env();
    mailer.outbox = Some(Default::default());
    Auth {
        origin: origin.into(),
        mailer,
    }
}

fn app(pool: PgPool) -> (Service, Arc<Mutex<Vec<String>>>) {
    app_with_auth(pool, test_auth("http://localhost:8000"))
}

fn app_with_auth(pool: PgPool, auth: Auth) -> (Service, Arc<Mutex<Vec<String>>>) {
    let outbox = auth.mailer.outbox.clone().unwrap();
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(pool))
            .hoop(affix_state::inject(auth))
            .push(routes())
            .push(crate::account::routes())
            .push(admin_routes()),
    );
    (service, outbox)
}

async fn post(app: &Service, path: &str, cookie: Option<&str>, form: &[(&str, &str)]) -> Response {
    let mut request = TestClient::post(format!("http://localhost:8000{path}"))
        .add_header("origin", "http://localhost:8000", true)
        .add_header("hx-request", "true", true)
        .form(&form);
    if let Some(cookie) = cookie {
        request = request.add_header("cookie", cookie, true);
    }
    request.send(app).await
}

async fn get(app: &Service, path: &str, cookie: Option<&str>) -> Response {
    let mut request = TestClient::get(format!("http://localhost:8000{path}")).add_header(
        "hx-request",
        "true",
        true,
    );
    if let Some(cookie) = cookie {
        request = request.add_header("cookie", cookie, true);
    }
    request.send(app).await
}

fn cookie(response: &Response) -> String {
    format!(
        "{SESSION_COOKIE}={}",
        response.cookie(SESSION_COOKIE).unwrap().value()
    )
}

fn mailed_token(outbox: &Mutex<Vec<String>>) -> String {
    let messages = outbox.lock().unwrap();
    let message = messages.last().unwrap();
    let (_, body) = message.split_once("\r\n\r\n").unwrap();
    let body = String::from_utf8(
        quoted_printable::decode(body, quoted_printable::ParseMode::Robust).unwrap(),
    )
    .unwrap();
    body.split("/auth/verify?token=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

async fn legacy_schema(pool: &PgPool) {
    for sql in [
        include_str!("../../migrations/0001_auth.sql"),
        include_str!("../../migrations/0002_admin.sql"),
        include_str!("../../migrations/0003_email_verification.sql"),
        include_str!("../../migrations/0004_school_emails.sql"),
    ] {
        sqlx::raw_sql(sql).execute(pool).await.unwrap();
    }
}

#[test]
fn argon2id_passwords_and_tokens() {
    let hash = hash_password(PASSWORD);
    assert!(hash.starts_with("$argon2id$v=19$"));
    assert!(verify_password(PASSWORD, &hash));
    assert!(!verify_password("wrong password", &hash));
    assert!(!verify_password("anything", "invalid hash"));
    assert_ne!(hash, hash_password(PASSWORD));
    let token = new_token();
    assert_eq!(hex::decode(&token).unwrap().len(), 32);
    assert_ne!(token, new_token());
    assert_ne!(token_hash(&token), token);
}

#[test]
fn cookies_and_email_normalization() {
    for (origin, secure) in [
        ("http://localhost:8000", false),
        ("https://bound.test", true),
    ] {
        let cookie = test_auth(origin).cookie("token".into());
        assert_eq!(cookie.http_only(), Some(true));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.secure(), Some(secure));
        assert_eq!(cookie.path(), Some("/"));
        assert_eq!(cookie.max_age(), Some(Duration::days(30)));
    }
    assert_eq!(
        normalize_email(" Student@DUKE.edu "),
        Some("student@duke.edu".into())
    );
    for email in ["", "invalid", "a@@duke.edu", "@duke.edu"] {
        assert!(normalize_email(email).is_none());
    }
}

#[tokio::test]
async fn forms_reject_unknown_domains_before_database_access_and_enforce_csrf() {
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(test_auth("http://localhost:8000")))
            .push(routes()),
    );
    for path in ["signup", "signin"] {
        let mut response = TestClient::get(format!("http://localhost:8000/auth/{path}"))
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        let html = response.take_string().await.unwrap();
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("School email"));
        assert_eq!(html.matches("type=\"email\"").count(), 1);
        assert!(!html.contains("name=\"school_email\""));
        assert!(!html.contains("aria-invalid=\"true\""));
        assert!(!html.contains("role=\"alert\""));
    }
    for path in ["signup", "signin", "verify", "resend", "signout"] {
        for origin in [None, Some("https://other.test")] {
            let mut request = TestClient::post(format!("http://localhost:8000/auth/{path}"));
            if let Some(origin) = origin {
                request = request.add_header("origin", origin, true);
            }
            assert_eq!(
                request.send(&service).await.status_code,
                Some(StatusCode::FORBIDDEN)
            );
        }
    }
    for email in [
        "invalid",
        "a@example.com",
        "a@unknown.edu",
        "a@sub.duke.edu",
        "a@duke.edu.evil.test",
        "a@@duke.edu",
    ] {
        let mut invalid = post(
            &service,
            "/auth/signup",
            None,
            &[("email", email), ("password", PASSWORD)],
        )
        .await;
        assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
        assert!(invalid.cookie(SESSION_COOKIE).is_none());
        let html = invalid.take_string().await.unwrap();
        assert!(html.contains("id=\"auth-panel\""));
        assert!(html.contains("id=\"auth-email-error\""));
        assert!(html.contains("aria-invalid=\"true\" aria-describedby=\"auth-email-error\""));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains(&format!("value=\"{email}\"")));
        if normalize_email(email).is_some() {
            assert!(html.contains("Please use an officially supported school email address."));
            assert!(html.contains("Supported domains: duke.edu."));
        } else {
            assert!(html.contains("Please enter a valid school email address."));
        }
        assert!(!html.contains("<!doctype"));
    }
    let mut short = post(
        &service,
        "/auth/signup",
        None,
        &[("email", "a@duke.edu"), ("password", "short")],
    )
    .await;
    assert_eq!(short.status_code, Some(StatusCode::BAD_REQUEST));
    let html = short.take_string().await.unwrap();
    assert!(html.contains("12 and 128"));
    assert!(html.contains("aria-describedby=\"auth-password-error\""));
    assert!(html.contains("value=\"a@duke.edu\""));
    assert!(!html.contains("value=\"short\""));
    let guest = get(&service, "/auth/verification", None).await;
    assert_eq!(guest.headers()["hx-redirect"], "/auth/signin");
    let unsupported_login = post(
        &service,
        "/auth/signin",
        None,
        &[("email", "a@example.com"), ("password", PASSWORD)],
    )
    .await;
    assert_eq!(
        unsupported_login.status_code,
        Some(StatusCode::UNAUTHORIZED)
    );
}

#[tokio::test]
async fn auth_gets_choose_the_requested_panel_and_only_wrap_normal_requests() {
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(test_auth("http://localhost:8000")))
            .push(routes()),
    );
    for (path, action, title) in [
        ("signup", "/auth/signup", "Create an account"),
        ("signin", "/auth/signin", "Sign in"),
        (
            "verify?token=from-email",
            "/auth/verify",
            "Verify your school email",
        ),
    ] {
        for htmx in [false, true] {
            let mut request = TestClient::get(format!("http://localhost:8000/auth/{path}"));
            if htmx {
                request = request.add_header("hx-request", "true", true);
            }
            let mut response = request.send(&service).await;
            assert_eq!(response.status_code, Some(StatusCode::OK));
            assert_eq!(
                response.headers()["content-type"],
                "text/html; charset=utf-8"
            );
            assert_eq!(response.headers()["vary"], "HX-Request");
            let html = response.take_string().await.unwrap();
            assert_eq!(html.contains("<!doctype html>"), !htmx);
            assert_eq!(html.matches("id=\"auth-panel\"").count(), 1);
            assert!(html.contains(title));
            assert!(html.contains(&format!("hx-post=\"{action}\"")));
            assert!(!html.contains("&lt;section"));
            if !htmx {
                assert!(html.contains(&format!("<title>{title} — Bound</title>")));
            }
        }
    }
}

#[tokio::test]
async fn signup_errors_render_full_pages_for_normal_forms_and_escape_email_values() {
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(test_auth("http://localhost:8000")))
            .push(routes()),
    );
    let secret = "do-not-echo-this-password";
    let mut response = TestClient::post("http://localhost:8000/auth/signup")
        .add_header("origin", "http://localhost:8000", true)
        .form(&[("email", "student@example.com"), ("password", secret)])
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    assert!(response.headers().get("hx-redirect").is_none());
    let html = response.take_string().await.unwrap();
    assert!(html.contains("<!doctype html>"));
    assert!(html.contains("Please use an officially supported school email address."));
    assert!(html.contains("value=\"student@example.com\""));
    assert!(html.contains("hx-post=\"/auth/signup\""));
    assert!(!html.contains(secret));

    let malicious_email = "\"><script>alert(1)</script>@example.com";
    let mut response = post(
        &service,
        "/auth/signup",
        None,
        &[("email", malicious_email), ("password", secret)],
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::BAD_REQUEST));
    let html = response.take_string().await.unwrap();
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("&#60;script&#62;"));
    assert!(!html.contains(secret));
}

#[tokio::test]
async fn signup_backend_failure_returns_a_retryable_form_without_internal_details() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://localhost/bound")
        .unwrap();
    pool.close().await;
    let (service, outbox) = app(pool);
    for htmx in [true, false] {
        let mut request = TestClient::post("http://localhost:8000/auth/signup")
            .add_header("origin", "http://localhost:8000", true)
            .form(&[("email", "student@duke.edu"), ("password", PASSWORD)]);
        if htmx {
            request = request.add_header("hx-request", "true", true);
        }
        let mut response = request.send(&service).await;
        assert_eq!(
            response.status_code,
            Some(StatusCode::INTERNAL_SERVER_ERROR)
        );
        assert!(response.cookie(SESSION_COOKIE).is_none());
        assert!(response.headers().get("hx-redirect").is_none());
        let html = response.take_string().await.unwrap();
        assert!(html.contains("complete your signup right now."));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("value=\"student@duke.edu\""));
        assert!(html.contains("hx-post=\"/auth/signup\""));
        assert_eq!(html.contains("<!doctype html>"), !htmx);
        assert!(!html.contains("pool timed out"));
        assert!(!html.contains("pool closed"));
        assert!(!html.contains(PASSWORD));
    }
    assert!(outbox.lock().unwrap().is_empty());
}

#[sqlx::test]
async fn signup_immediately_verifies_the_only_email_and_preserves_session_rules(pool: PgPool) {
    let (service, outbox) = app(pool.clone());
    let credentials = [
        ("email", " Student@DUKE.edu "),
        ("password", PASSWORD),
        ("is_admin", "true"),
        ("school_email", "someoneelse@duke.edu"),
        ("school", "fake"),
    ];
    let signup = post(&service, "/auth/signup", None, &credentials).await;
    assert_eq!(signup.headers()["hx-redirect"], "/auth/verification");
    let first_cookie = cookie(&signup);
    assert_eq!(outbox.lock().unwrap().len(), 1);
    assert!(outbox.lock().unwrap()[0].contains("To: student@duke.edu"));
    let state: (String, bool, bool) =
        sqlx::query_as("SELECT email, is_admin, email_verified_at IS NOT NULL FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, ("student@duke.edu".into(), false, false));
    assert_eq!(
        get(&service, "/account", Some(&first_cookie))
            .await
            .headers()["hx-redirect"],
        "/auth/verification"
    );
    let mut pending = get(&service, "/auth/verification", Some(&first_cookie)).await;
    let html = pending.take_string().await.unwrap();
    assert!(html.contains("hx-trigger=\"every 2s\""));
    assert!(html.contains("student@duke.edu"));
    assert!(!html.contains("type=\"email\""));
    assert!(html.contains("Sign out"));
    let token = mailed_token(&outbox);
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM email_verifications")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, token_hash(&token));
    assert_eq!(
        get(&service, "/auth/verification/status", Some(&first_cookie))
            .await
            .status_code,
        Some(StatusCode::NO_CONTENT)
    );
    assert_eq!(
        get(&service, "/auth/verification/status?user_id=1", None)
            .await
            .headers()["hx-redirect"],
        "/auth/signin"
    );

    let other_signup = post(
        &service,
        "/auth/signup",
        None,
        &[("email", "other@duke.edu"), ("password", PASSWORD)],
    )
    .await;
    let other_cookie = cookie(&other_signup);
    let mut link = TestClient::get(format!("http://localhost:8000/auth/verify?token={token}"))
        .send(&service)
        .await;
    assert!(
        link.take_string()
            .await
            .unwrap()
            .contains("hx-trigger=\"load, submit\"")
    );
    let verified: bool = sqlx::query_scalar(
        "SELECT email_verified_at IS NOT NULL FROM users WHERE email = 'student@duke.edu'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!verified, "GET must not consume a link");
    let mut confirmation = post(
        &service,
        "/auth/verify",
        Some(&other_cookie),
        &[("token", &token)],
    )
    .await;
    assert_eq!(confirmation.status_code, Some(StatusCode::OK));
    assert_eq!(cookie(&confirmation), other_cookie);
    assert!(
        confirmation.headers().get("set-cookie").is_none(),
        "Verification must not change the email tab's session"
    );
    assert!(
        confirmation
            .take_string()
            .await
            .unwrap()
            .contains("You can now close this page")
    );
    let state: (String, bool) = sqlx::query_as(
        "SELECT email, email_verified_at IS NOT NULL FROM users WHERE email = 'student@duke.edu'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, ("student@duke.edu".into(), true));
    assert_eq!(
        post(&service, "/auth/verify", None, &[("token", &token)])
            .await
            .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    assert_eq!(
        get(&service, "/auth/verification/status", Some(&first_cookie))
            .await
            .headers()["hx-redirect"],
        "/account"
    );
    assert_eq!(
        get(&service, "/auth/verification/status", Some(&other_cookie))
            .await
            .status_code,
        Some(StatusCode::NO_CONTENT)
    );
    let mut duplicate = post(&service, "/auth/signup", None, &credentials).await;
    assert_eq!(duplicate.status_code, Some(StatusCode::CONFLICT));
    let html = duplicate.take_string().await.unwrap();
    assert!(html.contains("Try signing in with this email instead."));
    assert!(html.contains("aria-describedby=\"auth-email-error\""));
    assert!(html.contains("value=\" Student@DUKE.edu \""));
    assert!(!html.contains(PASSWORD));
    assert_eq!(
        outbox.lock().unwrap().len(),
        2,
        "Duplicate signup must not send a link or overwrite credentials"
    );
    let wrong = post(
        &service,
        "/auth/signin",
        None,
        &[
            ("email", "student@duke.edu"),
            ("password", "wrong password"),
        ],
    )
    .await;
    assert_eq!(wrong.status_code, Some(StatusCode::UNAUTHORIZED));
    let signin = post(&service, "/auth/signin", Some(&first_cookie), &credentials).await;
    assert_eq!(signin.headers()["hx-redirect"], "/account");
    let second_cookie = cookie(&signin);
    assert_ne!(first_cookie, second_cookie);
    assert_eq!(
        get(&service, "/account", Some(&first_cookie))
            .await
            .headers()["hx-redirect"],
        "/auth/signin"
    );
    assert_eq!(
        get(&service, "/account", Some(&second_cookie))
            .await
            .status_code,
        Some(StatusCode::OK)
    );
    let logout = post(&service, "/auth/signout", Some(&second_cookie), &[]).await;
    assert_eq!(
        logout.cookie(SESSION_COOKIE).unwrap().max_age(),
        Some(Duration::ZERO)
    );
    assert_eq!(
        get(&service, "/account", Some(&second_cookie))
            .await
            .headers()["hx-redirect"],
        "/auth/signin"
    );
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(&service, "/auth/verification/status", Some(&other_cookie))
            .await
            .headers()["hx-redirect"],
        "/auth/signin"
    );
}

#[sqlx::test]
async fn resends_expiry_and_unverified_signin_use_the_same_email(pool: PgPool) {
    let (service, outbox) = app(pool.clone());
    let credentials = [("email", "student@duke.edu"), ("password", PASSWORD)];
    let signup = post(&service, "/auth/signup", None, &credentials).await;
    let first = cookie(&signup);
    let old = mailed_token(&outbox);
    post(
        &service,
        "/auth/resend",
        Some(&first),
        &[("email", "different@duke.edu")],
    )
    .await;
    assert_eq!(outbox.lock().unwrap().len(), 1);
    sqlx::query("UPDATE email_verifications SET expires_at = now() - interval '1 second', sent_at = now() - interval '2 minutes'")
        .execute(&pool).await.unwrap();
    assert_eq!(
        post(&service, "/auth/verify", None, &[("token", &old)])
            .await
            .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    let mut stopped = get(&service, "/auth/verification/status", Some(&first)).await;
    let html = stopped.take_string().await.unwrap();
    assert!(!html.contains("every 2s"), "Expired links stop polling");
    assert!(html.contains("Resend verification email"));
    assert!(!html.contains("type=\"email\""));
    let signin = post(&service, "/auth/signin", Some(&first), &credentials).await;
    let second = cookie(&signin);
    assert_eq!(
        get(&service, "/account", Some(&second)).await.headers()["hx-redirect"],
        "/auth/verification"
    );
    post(&service, "/auth/resend", Some(&second), &[]).await;
    let renewed = mailed_token(&outbox);
    assert_ne!(old, renewed);
    assert!(
        outbox
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("To: student@duke.edu")
    );
    assert_eq!(
        post(&service, "/auth/verify", None, &[("token", &old)])
            .await
            .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    assert_eq!(
        post(&service, "/auth/verify", None, &[("token", &renewed)])
            .await
            .status_code,
        Some(StatusCode::OK)
    );
    post(&service, "/auth/resend", Some(&second), &[]).await;
    assert_eq!(
        outbox.lock().unwrap().len(),
        2,
        "Verified users must not receive more verification mail"
    );
}

#[sqlx::test]
async fn failed_signup_delivery_keeps_a_session_and_can_be_retried(pool: PgPool) {
    let mut auth = test_auth("http://localhost:8000");
    auth.mailer.fail_delivery = true;
    let (failed_service, failed_outbox) = app_with_auth(pool.clone(), auth);
    let mut signup = post(
        &failed_service,
        "/auth/signup",
        None,
        &[("email", "student@duke.edu"), ("password", PASSWORD)],
    )
    .await;
    assert_eq!(signup.status_code, Some(StatusCode::SERVICE_UNAVAILABLE));
    assert!(signup.headers().get("hx-redirect").is_none());
    let session = cookie(&signup);
    let html = signup.take_string().await.unwrap();
    assert!(html.contains("Your account was created"));
    assert!(html.contains("Resend verification email"));
    assert!(!html.contains("type=\"email\""));
    assert!(failed_outbox.lock().unwrap().is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM email_verifications")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let mut waiting = get(&failed_service, "/auth/verification", Some(&session)).await;
    assert!(
        waiting
            .take_string()
            .await
            .unwrap()
            .contains("Resend verification email")
    );

    let (recovered_service, recovered_outbox) = app(pool.clone());
    let resent = post(&recovered_service, "/auth/resend", Some(&session), &[]).await;
    assert_eq!(resent.status_code, Some(StatusCode::OK));
    assert_eq!(recovered_outbox.lock().unwrap().len(), 1);
    let token = mailed_token(&recovered_outbox);
    assert_eq!(
        post(
            &recovered_service,
            "/auth/verify",
            None,
            &[("token", &token)]
        )
        .await
        .status_code,
        Some(StatusCode::OK)
    );
    assert_eq!(
        get(&recovered_service, "/account", Some(&session))
            .await
            .status_code,
        Some(StatusCode::OK)
    );
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(verify_password(PASSWORD, &hash));
}

#[sqlx::test]
async fn normal_forms_support_signup_verification_and_signin(pool: PgPool) {
    let (service, outbox) = app(pool);
    let credentials = [("email", "student@duke.edu"), ("password", PASSWORD)];
    let signup = TestClient::post("http://localhost:8000/auth/signup")
        .add_header("origin", "http://localhost:8000", true)
        .form(&credentials)
        .send(&service)
        .await;
    assert_eq!(signup.status_code, Some(StatusCode::SEE_OTHER));
    assert_eq!(signup.headers()["location"], "/auth/verification");
    let session = cookie(&signup);
    let mut pending = TestClient::get("http://localhost:8000/auth/verification")
        .add_header("cookie", &session, true)
        .send(&service)
        .await;
    assert!(
        pending
            .take_string()
            .await
            .unwrap()
            .contains("<!doctype html>")
    );
    let token = mailed_token(&outbox);
    let mut verification = TestClient::post("http://localhost:8000/auth/verify")
        .add_header("origin", "http://localhost:8000", true)
        .form(&[("token", token)])
        .send(&service)
        .await;
    assert_eq!(verification.status_code, Some(StatusCode::OK));
    assert!(
        verification
            .take_string()
            .await
            .unwrap()
            .contains("School email verified")
    );
    let signin = TestClient::post("http://localhost:8000/auth/signin")
        .add_header("origin", "http://localhost:8000", true)
        .form(&credentials)
        .send(&service)
        .await;
    assert_eq!(signin.status_code, Some(StatusCode::SEE_OTHER));
    assert_eq!(signin.headers()["location"], "/account");
}

#[sqlx::test]
async fn admin_requires_verified_supported_email_and_follows_role_changes(pool: PgPool) {
    let (service, _) = app(pool.clone());
    let user_id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ('student@duke.edu', 'unused') RETURNING id")
        .fetch_one(&pool).await.unwrap();
    let token = new_token();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(token_hash(&token)).bind(user_id).execute(&pool).await.unwrap();
    let cookie = format!("{SESSION_COOKIE}={token}");
    assert_eq!(
        get(&service, "/admin", None).await.headers()["hx-redirect"],
        "/auth/signin"
    );
    assert_eq!(
        get(&service, "/admin", Some(&cookie)).await.headers()["hx-redirect"],
        "/auth/verification"
    );
    sqlx::query("UPDATE users SET email_verified_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    for (is_admin, expected) in [
        (false, StatusCode::FORBIDDEN),
        (true, StatusCode::OK),
        (false, StatusCode::FORBIDDEN),
    ] {
        sqlx::query("UPDATE users SET is_admin = $1")
            .bind(is_admin)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            get(&service, "/admin", Some(&cookie)).await.status_code,
            Some(expected)
        );
    }
    sqlx::query("UPDATE users SET email = 'legacy@example.com', is_admin = true")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(&service, "/admin", Some(&cookie)).await.headers()["hx-redirect"],
        "/auth/verification"
    );
    let mut unsupported = get(&service, "/auth/verification", Some(&cookie)).await;
    let html = unsupported.take_string().await.unwrap();
    assert!(html.contains("School email required"));
    assert!(!html.contains("Resend verification email"));
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(&service, "/admin", Some(&cookie)).await.headers()["hx-redirect"],
        "/auth/signin"
    );
}

#[sqlx::test(migrations = false)]
async fn migrations_preserve_original_duke_verification_without_verifying_other_users(
    pool: PgPool,
) {
    for sql in [
        include_str!("../../migrations/0001_auth.sql"),
        include_str!("../../migrations/0002_admin.sql"),
        include_str!("../../migrations/0003_email_verification.sql"),
    ] {
        sqlx::raw_sql(sql).execute(&pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO users (email, password_hash, email_verified_at) VALUES
        ('verified@duke.edu', 'unused', now()), ('unverified@duke.edu', 'unused', NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0004_school_emails.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(SINGLE_EMAIL_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    let states: Vec<(String, bool)> =
        sqlx::query_as("SELECT email, email_verified_at IS NOT NULL FROM users ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        states,
        [
            ("verified@duke.edu".into(), true),
            ("unverified@duke.edu".into(), false)
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn migration_preserves_verified_identity_and_never_promotes_pending_addresses(pool: PgPool) {
    legacy_schema(&pool).await;
    sqlx::query("INSERT INTO users (email, password_hash, is_admin, school_email, school, school_email_verified_at) VALUES
        ('personal@example.com', 'retained hash', true, 'verified@duke.edu', 'duke', '2026-01-01T00:00:00Z'),
        ('pending@duke.edu', 'unused', false, 'pending@duke.edu', 'duke', NULL),
        ('mismatch@duke.edu', 'unused', false, NULL, NULL, NULL),
        ('legacy@example.com', 'unused', false, NULL, NULL, NULL)")
        .execute(&pool).await.unwrap();
    let user_id: i64 =
        sqlx::query_scalar("SELECT id FROM users WHERE email = 'personal@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let session_token = new_token();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(token_hash(&session_token)).bind(user_id).execute(&pool).await.unwrap();
    let matching_token = new_token();
    let mismatching_token = new_token();
    sqlx::query("INSERT INTO email_verifications (user_id, token_hash, expires_at, school_email, school)
        SELECT id, $1, now() + interval '30 minutes', 'pending@duke.edu', 'duke' FROM users WHERE email = 'pending@duke.edu'")
        .bind(token_hash(&matching_token)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO email_verifications (user_id, token_hash, expires_at, school_email, school)
        SELECT id, $1, now() + interval '30 minutes', 'different@duke.edu', 'duke' FROM users WHERE email = 'mismatch@duke.edu'")
        .bind(token_hash(&mismatching_token)).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO email_verifications (user_id, token_hash, expires_at, school_email, school)
        SELECT id, $1, now() + interval '30 minutes', 'unproven@duke.edu', 'duke' FROM users WHERE email = 'legacy@example.com'")
        .bind(token_hash(&new_token())).execute(&pool).await.unwrap();
    sqlx::raw_sql(SINGLE_EMAIL_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    let state: (i64, String, String, bool, bool) = sqlx::query_as(
        "SELECT id, email, password_hash, is_admin, email_verified_at = '2026-01-01T00:00:00Z' FROM users WHERE id = $1")
        .bind(user_id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        state,
        (
            user_id,
            "verified@duke.edu".into(),
            "retained hash".into(),
            true,
            true
        )
    );
    let legacy: (String, bool) = sqlx::query_as(
        "SELECT email, email_verified_at IS NOT NULL FROM users WHERE email = 'legacy@example.com'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(legacy, ("legacy@example.com".into(), false));
    let (service, _) = app(pool.clone());
    let session = format!("{SESSION_COOKIE}={session_token}");
    assert_eq!(
        get(&service, "/admin", Some(&session)).await.status_code,
        Some(StatusCode::OK)
    );
    assert_eq!(
        post(
            &service,
            "/auth/verify",
            None,
            &[("token", &mismatching_token)]
        )
        .await
        .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    assert_eq!(
        post(
            &service,
            "/auth/verify",
            None,
            &[("token", &matching_token)]
        )
        .await
        .status_code,
        Some(StatusCode::OK)
    );
    assert_eq!(
        post(
            &service,
            "/auth/verify",
            None,
            &[("token", &matching_token)]
        )
        .await
        .status_code,
        Some(StatusCode::BAD_REQUEST)
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 4);
    let mismatch_verified: bool = sqlx::query_scalar(
        "SELECT email_verified_at IS NOT NULL FROM users WHERE email = 'mismatch@duke.edu'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !mismatch_verified,
        "A link sent to another school email must not verify the canonical login"
    );
}

#[sqlx::test(migrations = false)]
async fn migration_rejects_login_ownership_conflicts_without_changing_accounts(pool: PgPool) {
    legacy_schema(&pool).await;
    // Even if the existing login's owner would move, never silently transfer it.
    sqlx::query("INSERT INTO users (email, password_hash, school_email, school, school_email_verified_at) VALUES
        ('first@example.com', 'first hash', 'second@duke.edu', 'duke', now()),
        ('second@duke.edu', 'second hash', 'third@duke.edu', 'duke', now())")
        .execute(&pool).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    let error = sqlx::raw_sql(SINGLE_EMAIL_MIGRATION)
        .execute(&mut *transaction)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("already another account's login email")
    );
    transaction.rollback().await.unwrap();
    let emails: Vec<String> = sqlx::query_scalar("SELECT email FROM users ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(emails, ["first@example.com", "second@duke.edu"]);
}

#[sqlx::test(migrations = false)]
async fn migration_rejects_inconsistent_verified_school_mapping(pool: PgPool) {
    legacy_schema(&pool).await;
    sqlx::query(
        "INSERT INTO users (email, password_hash, school_email, school, school_email_verified_at)
        VALUES ('personal@example.com', 'unused', 'student@duke.edu', 'wrong-school', now())",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut transaction = pool.begin().await.unwrap();
    let error = sqlx::raw_sql(SINGLE_EMAIL_MIGRATION)
        .execute(&mut *transaction)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("inconsistent verified school emails")
    );
    transaction.rollback().await.unwrap();
}
