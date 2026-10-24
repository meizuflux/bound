use super::*;
use salvo::test::{ResponseExt, TestClient};
use sha2::{Digest, Sha256};

async fn fixture(pool: &PgPool) -> (Service, String, i64) {
    let id: i64 = sqlx::query_scalar("INSERT INTO users(email,password_hash,email_verified_at) VALUES('profile@duke.edu','unused',now()) RETURNING id")
        .fetch_one(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')",
    )
    .bind(hex::encode(Sha256::digest(b"profile-test")))
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    let service = Service::new(
        Router::new()
            .hoop(affix_state::inject(pool.clone()))
            .hoop(affix_state::inject(auth::Auth::from_env()))
            .push(crate::account::routes())
            .push(auth::routes()),
    );
    (service, "bound_session=profile-test".into(), id)
}

async fn post(
    service: &Service,
    cookie: &str,
    path: &str,
    htmx: bool,
    fields: &[(&str, &str)],
) -> Response {
    let mut req = TestClient::post(format!("http://localhost:8000{path}"))
        .add_header("origin", "http://localhost:8000", true)
        .add_header("cookie", cookie, true)
        .form(&fields);
    if htmx {
        req = req.add_header("hx-request", "true", true);
    }
    req.send(service).await
}

#[sqlx::test]
async fn optional_profile_preserves_privacy_and_prevents_stale_updates(pool: PgPool) {
    let (service, cookie, id) = fixture(&pool).await;
    let mut page = TestClient::get("http://localhost:8000/account/profile?onboarding=true")
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    let html = page.take_string().await.unwrap();
    assert!(html.contains("Complete your profile"));
    assert!(html.contains("Skip for now"));
    assert!(html.contains("name=\"profile_visible\" checked"));
    assert!(!html.contains("name=\"share_phone\" checked"));
    let fields = [
        ("display_name", "<script>Alice</script>"),
        ("phone_number", "+1 919 555 0123"),
        ("profile_visible", "on"),
        ("revision", "1"),
    ];
    let saved = post(&service, &cookie, "/account/profile", true, &fields).await;
    assert_eq!(saved.headers()["hx-redirect"], "/account");
    let state: (String, bool, bool) = sqlx::query_as(
        "SELECT display_name, share_phone, profile_onboarding_done FROM users WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, ("<script>Alice</script>".into(), false, true));
    let mut page = TestClient::get("http://localhost:8000/account/profile")
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    let html = page.take_string().await.unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&#60;script&#62;"));
    assert_eq!(
        post(&service, &cookie, "/account/profile", false, &fields)
            .await
            .status_code,
        Some(StatusCode::CONFLICT)
    );
    let saved = post(
        &service,
        &cookie,
        "/account/profile",
        false,
        &[("revision", "2")],
    )
    .await;
    assert_eq!(saved.headers()["location"], "/account");
    let state: (String, String, bool, bool) = sqlx::query_as(
        "SELECT display_name, phone_number, profile_visible, share_phone FROM users WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, (String::new(), String::new(), false, false));
    for (revision, share) in [("3", true), ("4", false)] {
        let mut fields = vec![
            ("revision", revision),
            ("phone_number", "+1 919 555 0123"),
            ("profile_visible", "on"),
        ];
        if share {
            fields.push(("share_phone", "on"));
        }
        let response = post(&service, &cookie, "/account/profile", true, &fields).await;
        assert_eq!(response.headers()["hx-redirect"], "/account");
        let sharing: bool = sqlx::query_scalar("SELECT share_phone FROM users WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sharing, share);
    }
    let details: String = sqlx::query_scalar(
        "SELECT details FROM activity_events WHERE event_type='profile_updated' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!details.contains("919"));
    assert!(!details.contains("Alice"));
}

#[sqlx::test]
async fn skip_and_validation_support_normal_and_htmx_forms(pool: PgPool) {
    let (service, cookie, id) = fixture(&pool).await;
    for htmx in [false, true] {
        let mut bad = post(
            &service,
            &cookie,
            "/account/profile",
            htmx,
            &[
                ("revision", "1"),
                ("phone_number", "bad"),
                ("onboarding", "true"),
            ],
        )
        .await;
        assert_eq!(bad.status_code, Some(StatusCode::BAD_REQUEST));
        let html = bad.take_string().await.unwrap();
        assert_eq!(html.contains("<!doctype html>"), !htmx);
        assert!(html.contains("aria-invalid=\"true\""));
        assert!(html.contains("Complete your profile"));
        assert_eq!(
            post(
                &service,
                &cookie,
                "/account/profile",
                htmx,
                &[("revision", "1"), ("share_phone", "on")]
            )
            .await
            .status_code,
            Some(StatusCode::BAD_REQUEST)
        );
        let skipped = post(&service, &cookie, "/account/profile/skip", htmx, &[]).await;
        assert_eq!(
            skipped.headers()[if htmx { "hx-redirect" } else { "location" }],
            "/account"
        );
    }
    let state: (String, bool, bool, bool) = sqlx::query_as("SELECT display_name, profile_visible, share_phone, profile_onboarding_done FROM users WHERE id=$1").bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(state, (String::new(), true, false, true));
    let response = TestClient::get("http://localhost:8000/auth/verification/status")
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert_eq!(response.headers()["location"], "/account");
    let bad_origin = TestClient::post("http://localhost:8000/account/profile/skip")
        .add_header("origin", "https://other.example", true)
        .add_header("cookie", &cookie, true)
        .send(&service)
        .await;
    assert_eq!(bad_origin.status_code, Some(StatusCode::FORBIDDEN));
    sqlx::query("UPDATE users SET email_verified_at=NULL WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let denied = post(
        &service,
        &cookie,
        "/account/profile",
        false,
        &[("revision", "1")],
    )
    .await;
    assert_eq!(denied.headers()["location"], "/auth/verification");
}
