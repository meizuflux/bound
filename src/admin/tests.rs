use salvo::test::{ResponseExt, TestClient};
use sha2::{Digest, Sha256};

use super::*;

fn app(pool: PgPool) -> Service {
    Service::new(
        Router::new()
            .hoop(affix_state::inject(pool))
            .hoop(affix_state::inject(auth::Auth::from_env()))
            .push(routes()),
    )
}

async fn admin_session(pool: &PgPool) -> (i64, String) {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash, is_admin, email_verified_at)
        VALUES ('admin@duke.edu', 'SECRET_PASSWORD_HASH', true, now()) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(hex::encode(Sha256::digest(b"SECRET_SESSION_TOKEN"))).bind(id).execute(pool).await.unwrap();
    (id, "bound_session=SECRET_SESSION_TOKEN".into())
}

async fn get(app: &Service, path: &str, cookie: Option<&str>, htmx: bool) -> Response {
    let mut request = TestClient::get(format!("http://localhost:8000{path}"));
    if let Some(cookie) = cookie {
        request = request.add_header("cookie", cookie, true);
    }
    if htmx {
        request = request.add_header("hx-request", "true", true);
    }
    request.send(app).await
}

#[sqlx::test]
async fn every_admin_route_requires_current_verified_admin_permissions(pool: PgPool) {
    let (id, cookie) = admin_session(&pool).await;
    let service = app(pool.clone());
    let paths = [
        "/admin".to_owned(),
        "/admin/users".into(),
        format!("/admin/users/{id}"),
        "/admin/activity".into(),
        "/admin/users/new".into(),
        "/admin/flights".into(),
        "/admin/flights/new".into(),
        "/admin/corrections".into(),
    ];
    for htmx in [false, true] {
        for path in &paths {
            let guest = get(&service, path, None, htmx).await;
            assert_eq!(
                guest.headers()[if htmx { "hx-redirect" } else { "location" }],
                "/auth/signin"
            );
            let mut response = get(&service, path, Some(&cookie), htmx).await;
            assert_eq!(response.status_code, Some(StatusCode::OK));
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["vary"], "HX-Request");
            let html = response.take_string().await.unwrap();
            assert_eq!(html.contains("<!doctype html>"), !htmx);
            assert_eq!(html.matches("id=\"admin-panel\"").count(), 1);
            assert!(html.contains("href=\"/admin/flights\""));
            assert!(!html.contains("SECRET_PASSWORD_HASH"));
            assert!(!html.contains("SECRET_SESSION_TOKEN"));
        }
    }
    for (is_admin, verified, email, expected) in [
        (false, true, "admin@duke.edu", StatusCode::FORBIDDEN),
        (true, false, "admin@duke.edu", StatusCode::SEE_OTHER),
        (true, true, "legacy@example.com", StatusCode::SEE_OTHER),
    ] {
        sqlx::query("UPDATE users SET is_admin = $2, email_verified_at = CASE WHEN $3 THEN now() ELSE NULL END, email = $4 WHERE id = $1")
            .bind(id).bind(is_admin).bind(verified).bind(email).execute(&pool).await.unwrap();
        for path in &paths {
            assert_eq!(
                get(&service, path, Some(&cookie), false).await.status_code,
                Some(expected)
            );
        }
    }
}

#[sqlx::test]
async fn users_search_and_pagination_are_bounded_and_literal(pool: PgPool) {
    let (_, cookie) = admin_session(&pool).await;
    sqlx::query("INSERT INTO users (email, password_hash) SELECT 'student' || n || '@duke.edu', 'unused' FROM generate_series(1, 26) n")
        .execute(&pool).await.unwrap();
    let service = app(pool);
    let mut first = get(&service, "/admin/users?q=STUDENT", Some(&cookie), false).await;
    let html = first.take_string().await.unwrap();
    assert!(html.contains("student26@duke.edu"));
    assert!(!html.contains(">student1@duke.edu<"));
    assert!(!html.contains(">admin@duke.edu<"));
    assert!(html.contains("name=\"q\" value=\"STUDENT\""));
    assert!(html.contains(">Next</button>"));
    let mut second = get(
        &service,
        "/admin/users?q=STUDENT&page=2",
        Some(&cookie),
        false,
    )
    .await;
    let html = second.take_string().await.unwrap();
    assert!(html.contains("student1@duke.edu"));
    assert!(!html.contains(">Next</button>"));
    assert!(html.contains(">Previous</button>"));
    let mut literal = get(&service, "/admin/users?q=%25", Some(&cookie), false).await;
    assert!(
        literal
            .take_string()
            .await
            .unwrap()
            .contains("No users match")
    );
    for path in [
        "/admin/users?page=0",
        "/admin/users?page=-1",
        "/admin/activity?page=invalid",
        "/admin/activity?page=9223372036854775807",
    ] {
        assert_eq!(
            get(&service, path, Some(&cookie), false).await.status_code,
            Some(StatusCode::BAD_REQUEST)
        );
    }
}

#[sqlx::test]
async fn user_details_and_history_escape_values_and_unsupported_writes(pool: PgPool) {
    let (_, cookie) = admin_session(&pool).await;
    let payload = "<script>alert(1)</script>@example.com";
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ($1, 'unused') RETURNING id",
    )
    .bind(payload)
    .fetch_one(&pool)
    .await
    .unwrap();
    let service = app(pool);
    for path in [
        "/admin/users".to_owned(),
        format!("/admin/users/{id}"),
        "/admin/activity".into(),
    ] {
        let mut response = get(&service, &path, Some(&cookie), false).await;
        let html = response.take_string().await.unwrap();
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&#60;script&#62;"));
        let response = TestClient::post(format!("http://localhost:8000{path}"))
            .add_header("cookie", &cookie, true)
            .add_header("origin", "http://localhost:8000", true)
            .send(&service)
            .await;
        assert_eq!(
            response.status_code,
            Some(if path == "/admin/users" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::METHOD_NOT_ALLOWED
            })
        );
    }
    for path in [
        "/admin/users/0",
        "/admin/users/nope",
        "/admin/users/9223372036854775807",
    ] {
        assert_eq!(
            get(&service, path, Some(&cookie), false).await.status_code,
            Some(StatusCode::NOT_FOUND)
        );
    }
}

#[sqlx::test]
async fn activity_tracks_committed_changes_and_preserves_history_after_deletion(pool: PgPool) {
    let (_, cookie) = admin_session(&pool).await;
    let id: i64 = sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ('student@duke.edu', 'unused') RETURNING id")
        .fetch_one(&pool).await.unwrap();
    sqlx::query("UPDATE users SET email_verified_at = now(), is_admin = true WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    // No-op updates and rolled-back changes must not produce misleading events.
    sqlx::query("UPDATE users SET is_admin = true WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query("UPDATE users SET is_admin = false WHERE id = $1")
        .bind(id)
        .execute(&mut *transaction)
        .await
        .unwrap();
    transaction.rollback().await.unwrap();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ('HIDDEN_HASH', $1, now() + interval '1 day')")
        .bind(id).execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let events: Vec<String> =
        sqlx::query_scalar("SELECT event_type FROM activity_events WHERE user_id = $1 ORDER BY id")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        events,
        [
            "account_created",
            "email_verified",
            "admin_granted",
            "session_started",
            "session_ended"
        ]
    );
    let service = app(pool.clone());
    let mut response = get(&service, "/admin/activity?q=student", Some(&cookie), false).await;
    let html = response.take_string().await.unwrap();
    assert!(html.contains("Email verified"));
    assert!(html.contains("Admin access granted"));
    assert!(!html.contains(">admin@duke.edu<"));
    assert!(!html.contains("HIDDEN_HASH"));
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let history: (i64, bool) = sqlx::query_as("SELECT count(*), bool_and(user_id IS NULL) FROM activity_events WHERE email = 'student@duke.edu'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(history, (6, true));
    let mut response = get(&service, "/admin/activity?q=student", Some(&cookie), false).await;
    let html = response.take_string().await.unwrap();
    assert!(html.contains("Account deleted"));
    assert!(!html.contains(&format!("href=\"/admin/users/{id}\"")));
}

#[sqlx::test(migrations = false)]
async fn enabling_activity_preserves_existing_accounts_without_inventing_history(pool: PgPool) {
    for migration in [
        include_str!("../../migrations/0001_auth.sql"),
        include_str!("../../migrations/0002_admin.sql"),
        include_str!("../../migrations/0003_email_verification.sql"),
        include_str!("../../migrations/0004_school_emails.sql"),
        include_str!("../../migrations/0005_single_school_email.sql"),
    ] {
        sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash, is_admin, email_verified_at)
        VALUES ('existing@duke.edu', 'retained hash', true, now()) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0006_activity_events.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let account: (String, String, bool, bool) = sqlx::query_as(
        "SELECT email, password_hash, is_admin, email_verified_at IS NOT NULL FROM users WHERE id = $1")
        .bind(id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        account,
        (
            "existing@duke.edu".into(),
            "retained hash".into(),
            true,
            true
        )
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM activity_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    // Preservation is tested against the original schema; current pages require later migrations.
    sqlx::raw_sql(include_str!("../../migrations/0007_flights.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/0008_activity_attribution.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0009_admin_users.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let service = app(pool.clone());
    let (_, cookie) = admin_session(&pool).await;
    let mut response = get(&service, "/admin/activity?q=existing", Some(&cookie), false).await;
    assert!(
        response
            .take_string()
            .await
            .unwrap()
            .contains("No recorded activity found.")
    );
    sqlx::query("UPDATE users SET is_admin = false WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let event: String =
        sqlx::query_scalar("SELECT event_type FROM activity_events WHERE user_id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(event, "admin_revoked");
}

#[sqlx::test]
async fn admin_user_writes_are_versioned_attributed_and_protect_self(pool: PgPool) {
    let (actor, _) = admin_session(&pool).await;
    let id = mutations::create(&pool, actor, "new@duke.edu", "HASH_NOT_LOGGED", false)
        .await
        .unwrap();
    let user = data::user(&pool, id).await.unwrap().unwrap();
    assert!(!user.verified);
    mutations::update(&pool, actor, id, user.version, true, true, false)
        .await
        .unwrap();
    assert!(matches!(
        mutations::update(&pool, actor, id, user.version, false, false, false).await,
        Err(mutations::WriteError::Conflict(_))
    ));
    let own = data::user(&pool, actor).await.unwrap().unwrap();
    assert!(matches!(
        mutations::update(&pool, actor, actor, own.version, false, false, false).await,
        Err(mutations::WriteError::Conflict(_))
    ));
    assert!(matches!(
        mutations::delete(&pool, actor, actor, own.version).await,
        Err(mutations::WriteError::Conflict(_))
    ));
    let target = data::user(&pool, id).await.unwrap().unwrap();
    mutations::delete(&pool, actor, id, target.version)
        .await
        .unwrap();
    let events: Vec<(Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT actor_id,actor_email FROM activity_events WHERE email='new@duke.edu'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(events.len() >= 4);
    assert!(
        events
            .iter()
            .all(|(id, email)| *id == Some(actor) && email.as_deref() == Some("admin@duke.edu"))
    );
    assert!(data::user(&pool, id).await.unwrap().is_none());
}

#[sqlx::test]
async fn concurrent_user_updates_cannot_both_overwrite_same_revision(pool: PgPool) {
    let (actor, _) = admin_session(&pool).await;
    let id = mutations::create(&pool, actor, "concurrent@duke.edu", "unused", false)
        .await
        .unwrap();
    let version = data::user(&pool, id).await.unwrap().unwrap().version;
    let (left, right) = tokio::join!(
        mutations::update(&pool, actor, id, version, true, false, false),
        mutations::update(&pool, actor, id, version, false, true, false)
    );
    assert_ne!(left.is_ok(), right.is_ok());
}

#[sqlx::test]
async fn admin_post_origin_confirmation_and_live_permissions_are_enforced(pool: PgPool) {
    let (actor, cookie) = admin_session(&pool).await;
    let target = mutations::create(&pool, actor, "target@duke.edu", "unused", false)
        .await
        .unwrap();
    let service = app(pool.clone());
    let version = data::user(&pool, target).await.unwrap().unwrap().version;
    let response = TestClient::post(format!("http://localhost:8000/admin/users/{target}/delete"))
        .add_header("cookie", &cookie, true)
        .add_header("origin", "https://attacker.example", true)
        .body(format!("version={version}&confirm_email=target%40duke.edu"))
        .add_header("content-type", "application/x-www-form-urlencoded", true)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
    assert!(data::user(&pool, target).await.unwrap().is_some());
    let mut response =
        TestClient::post(format!("http://localhost:8000/admin/users/{target}/delete"))
            .add_header("cookie", &cookie, true)
            .add_header("origin", "http://localhost:8000", true)
            .body(format!("version={version}&confirm_email=wrong%40duke.edu"))
            .add_header("content-type", "application/x-www-form-urlencoded", true)
            .send(&service)
            .await;
    assert_eq!(response.status_code, Some(StatusCode::CONFLICT));
    assert!(
        response
            .take_string()
            .await
            .unwrap()
            .contains("confirm deletion")
    );
    assert!(data::user(&pool, target).await.unwrap().is_some());
    sqlx::query("UPDATE users SET is_admin=false WHERE id=$1")
        .bind(actor)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        mutations::update(&pool, actor, target, version, false, false, false).await,
        Err(mutations::WriteError::Conflict(_))
    ));
}

async fn post_form(
    service: &Service,
    path: &str,
    cookie: &str,
    body: String,
    htmx: bool,
) -> Response {
    let mut request = TestClient::post(format!("http://localhost:8000{path}"))
        .add_header("cookie", cookie, true)
        .add_header("origin", "http://localhost:8000", true)
        .add_header("content-type", "application/x-www-form-urlencoded", true)
        .body(body);
    if htmx {
        request = request.add_header("hx-request", "true", true);
    }
    request.send(service).await
}

#[sqlx::test]
async fn flight_admin_forms_create_update_archive_restore_and_merge(pool: PgPool) {
    let (actor, cookie) = admin_session(&pool).await;
    let service = app(pool.clone());
    let input = "flight_number=AA123&departure_date=2027-04-02&departure_time=15%3A30&origin=RDU&destination=JFK";
    let response = post_form(&service, "/admin/flights", &cookie, input.into(), true).await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let id: i64 = sqlx::query_scalar("SELECT id FROM flights WHERE flight_number='AA123'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        response.headers()["hx-location"]
            .to_str()
            .unwrap()
            .contains(&format!("/admin/flights/{id}"))
    );
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM flights WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut response = get(
        &service,
        &format!("/admin/flights/{id}"),
        Some(&cookie),
        true,
    )
    .await;
    let html = response.take_string().await.unwrap();
    assert!(html.contains("Scheduled departure at origin"));
    assert!(!html.contains("<!doctype html>"));
    let response = post_form(
        &service,
        &format!("/admin/flights/{id}"),
        &cookie,
        format!("{input}&revision={revision}"),
        false,
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
    let mut stale = post_form(
        &service,
        &format!("/admin/flights/{id}"),
        &cookie,
        format!("{input}&revision={revision}"),
        true,
    )
    .await;
    assert_eq!(stale.status_code, Some(StatusCode::CONFLICT));
    assert!(
        stale
            .take_string()
            .await
            .unwrap()
            .contains("Reload current details")
    );
    let response = post_form(
        &service,
        &format!("/admin/flights/{id}/aliases"),
        &cookie,
        "number=DL456".into(),
        true,
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM flights WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let response = post_form(
        &service,
        &format!("/admin/flights/{id}/status"),
        &cookie,
        format!("revision={revision}&deleted=true&confirm=on"),
        true,
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let mut archived = get(
        &service,
        &format!("/admin/flights/{id}"),
        Some(&cookie),
        true,
    )
    .await;
    let archived_html = archived.take_string().await.unwrap();
    assert!(archived_html.contains("Restore flight"));
    assert!(!archived_html.contains(">Save flight</button>"));
    assert!(!archived_html.contains(">Add alternate number</button>"));
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM flights WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(deleted);
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM flights WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let response = post_form(
        &service,
        &format!("/admin/flights/{id}/status"),
        &cookie,
        format!("revision={revision}&deleted=false&confirm=on"),
        true,
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM flights WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let invalid_input = input.replace("15%3A30", "25%3A99");
    let mut invalid = post_form(
        &service,
        &format!("/admin/flights/{id}"),
        &cookie,
        format!("{invalid_input}&revision={revision}"),
        true,
    )
    .await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
    let invalid_html = invalid.take_string().await.unwrap();
    assert!(invalid_html.contains("DL456"));
    assert!(invalid_html.contains("aria-invalid=\"true\""));
    let target = crate::flights::data::admin_create(
        &pool,
        actor,
        &crate::flights::data::FlightInput {
            flight_number: "AA124".into(),
            departure_date: "2027-04-02".into(),
            departure_time: "15:30".into(),
            origin: "RDU".into(),
            destination: "JFK".into(),
        },
        false,
    )
    .await
    .unwrap()
    .flight_id;
    let mut preview = get(
        &service,
        &format!("/admin/flights/{id}/merge?target={target}"),
        Some(&cookie),
        false,
    )
    .await;
    assert!(
        preview
            .take_string()
            .await
            .unwrap()
            .contains("I checked both records")
    );
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM flights WHERE id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let response = post_form(
        &service,
        &format!("/admin/flights/{id}/merge"),
        &cookie,
        format!("source_revision={revision}&target_revision=1&target_id={target}&confirm=on"),
        true,
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let destination: Option<i64> =
        sqlx::query_scalar("SELECT merged_into FROM flights WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(destination, Some(target));
}
