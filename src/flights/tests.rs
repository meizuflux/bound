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

async fn session(pool: &PgPool, email: &str) -> (i64, String) {
    let id: i64 = sqlx::query_scalar("INSERT INTO users(email,password_hash,email_verified_at) VALUES($1,'SECRET_HASH',now()) RETURNING id")
        .bind(email).fetch_one(pool).await.unwrap();
    let token = format!("flight-test-{id}");
    sqlx::query(
        "INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')",
    )
    .bind(hex::encode(Sha256::digest(token.as_bytes())))
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    (id, format!("bound_session={token}"))
}

async fn get(
    service: &Service,
    path: &str,
    cookie: Option<&str>,
    target: Option<&str>,
) -> Response {
    let mut request = TestClient::get(format!("http://localhost:8000{path}"));
    if let Some(cookie) = cookie {
        request = request.add_header("cookie", cookie, true);
    }
    if let Some(target) = target {
        request =
            request
                .add_header("hx-request", "true", true)
                .add_header("hx-target", target, true);
    }
    request.send(service).await
}

async fn post(
    service: &Service,
    path: &str,
    cookie: &str,
    htmx: bool,
    values: &[(&str, &str)],
) -> Response {
    let mut request = TestClient::post(format!("http://localhost:8000{path}"))
        .add_header("origin", "http://localhost:8000", true)
        .add_header("cookie", cookie, true)
        .form(&values);
    if htmx {
        request = request.add_header("hx-request", "true", true);
    }
    request.send(service).await
}

fn input() -> data::FlightInput {
    data::FlightInput {
        flight_number: "AA123".into(),
        departure_date: "2027-03-20".into(),
        departure_time: "15:30".into(),
        origin: "RDU".into(),
        destination: "BOS".into(),
    }
}

fn form() -> [(&'static str, &'static str); 6] {
    [
        ("airline", "AA"),
        ("number", "123"),
        ("departure_date", "2027-03-20"),
        ("departure_time", "15:30"),
        ("origin", "RDU"),
        ("destination", "BOS"),
    ]
}

#[sqlx::test]
async fn every_flight_route_requires_verified_account_and_post_origin(pool: PgPool) {
    let (user_id, cookie) = session(&pool, "student@duke.edu").await;
    let service = app(pool.clone());
    for path in [
        "/flights",
        "/flights/mine",
        "/flights/new",
        "/flights/1",
        "/flights/1/status",
    ] {
        assert_eq!(
            get(&service, path, None, None).await.headers()["location"],
            "/auth/signin"
        );
    }
    for path in [
        "/flights/new/review",
        "/flights/new/edit",
        "/flights/new/create",
        "/flights/1/join",
        "/flights/1/leave",
        "/flights/1/confirm",
        "/flights/1/corrections",
    ] {
        let response = TestClient::post(format!("http://localhost:8000{path}"))
            .add_header("cookie", &cookie, true)
            .send(&service)
            .await;
        assert_eq!(response.status_code, Some(StatusCode::FORBIDDEN));
    }
    sqlx::query("UPDATE users SET email_verified_at=NULL WHERE id=$1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
    for path in ["/flights", "/flights/mine", "/flights/new"] {
        assert_eq!(
            get(&service, path, Some(&cookie), None).await.headers()["location"],
            "/auth/verification"
        );
    }
}

#[sqlx::test]
async fn search_swaps_results_only_and_never_shows_member_identity(pool: PgPool) {
    let (user_id, cookie) = session(&pool, "private-student@duke.edu").await;
    data::create(&pool, user_id, &input()).await.unwrap();
    let service = app(pool);
    let path = "/flights?number=aa%20123&date=2027-03-20";
    for target in [None, Some("div#flight-results")] {
        let mut response = get(&service, path, Some(&cookie), target).await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        assert_eq!(response.headers()["cache-control"], "no-store");
        let html = response.take_string().await.unwrap();
        assert_eq!(html.contains("<!doctype html>"), target.is_none());
        assert_eq!(
            html.contains("id=\"flight-search-number\""),
            target.is_none()
        );
        assert!(html.contains("AA123"));
        assert!(html.contains("1 from your school"));
        assert!(!html.contains("private-student@duke.edu"));
        assert!(!html.contains("SECRET_HASH"));
    }
    let mut invalid = get(
        &service,
        "/flights?number=%3Cscript%3E&date=2027-03-20",
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
    let html = invalid.take_string().await.unwrap();
    assert!(!html.contains("<SCRIPT>"));
    assert!(html.contains("&#60;SCRIPT&#62;"));
}

#[sqlx::test]
async fn review_creation_duplicate_and_membership_work_with_normal_forms(pool: PgPool) {
    let (user_id, cookie) = session(&pool, "student@duke.edu").await;
    let service = app(pool.clone());
    let mut review_response = post(&service, "/flights/new/review", &cookie, false, &form()).await;
    assert_eq!(review_response.status_code, Some(StatusCode::OK));
    let html = review_response.take_string().await.unwrap();
    assert!(html.contains("Check your flight"));
    assert!(html.contains("America/New_York"));
    assert!(html.contains("Create and add to my flights"));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM flights")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let response = post(&service, "/flights/new/create", &cookie, false, &form()).await;
    assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
    let flight_id: i64 = sqlx::query_scalar("SELECT id FROM flights")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        response.headers()["location"],
        format!("/flights/{flight_id}?notice=created")
    );
    assert!(
        data::flight(&pool, user_id, flight_id)
            .await
            .unwrap()
            .unwrap()
            .joined
    );
    let (_, other_cookie) = session(&pool, "other@duke.edu").await;
    let mut other_form = form();
    other_form[3] = ("departure_time", "16:30");
    let response = post(
        &service,
        "/flights/new/create",
        &other_cookie,
        true,
        &other_form,
    )
    .await;
    assert_eq!(
        response.headers()["hx-redirect"],
        format!("/flights/{flight_id}?notice=existing")
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM flights")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let response = post(
        &service,
        &format!("/flights/{flight_id}/join"),
        &other_cookie,
        false,
        &[],
    )
    .await;
    assert_eq!(
        response.headers()["location"],
        format!("/flights/{flight_id}")
    );
    let response = post(
        &service,
        &format!("/flights/{flight_id}/leave"),
        &other_cookie,
        false,
        &[],
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
}

#[sqlx::test]
async fn correction_errors_preserve_escaped_values_and_polling_stops_after_review(pool: PgPool) {
    let (user_id, cookie) = session(&pool, "student@duke.edu").await;
    let outcome = data::create(&pool, user_id, &input()).await.unwrap();
    let id = outcome.flight_id;
    let service = app(pool.clone());
    let mut invalid = post(
        &service,
        &format!("/flights/{id}/corrections"),
        &cookie,
        true,
        &[
            ("revision", "1"),
            ("field", "departure_time"),
            ("value", "<script>"),
            ("reason", "<b>ticket</b>"),
        ],
    )
    .await;
    assert_eq!(invalid.status_code, Some(StatusCode::BAD_REQUEST));
    let html = invalid.take_string().await.unwrap();
    assert!(!html.contains("<script>"));
    assert!(html.contains("&#60;script&#62;"));
    assert!(html.contains("&#60;b&#62;ticket&#60;/b&#62;"));
    let mut response = post(
        &service,
        &format!("/flights/{id}/corrections"),
        &cookie,
        true,
        &[
            ("revision", "1"),
            ("field", "departure_time"),
            ("value", "16:30"),
            ("reason", "My ticket"),
        ],
    )
    .await;
    assert!(response.headers().get("hx-redirect").is_none());
    let html = response.take_string().await.unwrap();
    assert!(html.contains("id=\"flights-panel\""));
    assert!(html.contains("Your correction was sent for review"));
    assert!(!html.contains("<!doctype html>"));
    let mut pending = get(
        &service,
        &format!("/flights/{id}/status"),
        Some(&cookie),
        Some("div#flight-summary"),
    )
    .await;
    let html = pending.take_string().await.unwrap();
    assert!(html.contains("every 15s"));
    assert!(html.contains("15:30"));
    assert!(html.contains("16:30"));
    assert!(!html.contains("id=\"flight-correction\""));
    sqlx::query("UPDATE flight_corrections SET status='rejected' WHERE flight_id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let mut resolved = get(
        &service,
        &format!("/flights/{id}/status"),
        Some(&cookie),
        Some("div#flight-summary"),
    )
    .await;
    assert!(!resolved.take_string().await.unwrap().contains("every 15s"));
    assert_eq!(
        get(
            &service,
            &format!("/flights/{id}/status"),
            Some(&cookie),
            None
        )
        .await
        .headers()["location"],
        format!("/flights/{id}")
    );
}

#[sqlx::test]
async fn confirmation_is_explicit_and_mutations_return_small_fragments(pool: PgPool) {
    let (user_id, cookie) = session(&pool, "student@duke.edu").await;
    let id = data::create(&pool, user_id, &input())
        .await
        .unwrap()
        .flight_id;
    assert!(
        !data::flight(&pool, user_id, id)
            .await
            .unwrap()
            .unwrap()
            .confirmed
    );
    let service = app(pool.clone());
    let mut response = post(
        &service,
        &format!("/flights/{id}/confirm"),
        &cookie,
        true,
        &[("revision", "1")],
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let html = response.take_string().await.unwrap();
    assert!(html.starts_with("<div id=\"flight-summary\""));
    assert!(!html.contains("<!doctype html>"));
    assert!(
        data::flight(&pool, user_id, id)
            .await
            .unwrap()
            .unwrap()
            .confirmed
    );
    let mut response = post(
        &service,
        &format!("/flights/{id}/confirm"),
        &cookie,
        true,
        &[("revision", "999")],
    )
    .await;
    assert_eq!(response.status_code, Some(StatusCode::CONFLICT));
    assert!(
        response
            .take_string()
            .await
            .unwrap()
            .contains("role=\"alert\"")
    );
    for path in [
        "/flights?page=0",
        "/flights/mine?page=-1",
        "/flights/0",
        "/flights/not-an-id",
    ] {
        let response = get(&service, path, Some(&cookie), None).await;
        assert!(matches!(
            response.status_code,
            Some(StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND)
        ));
    }
}

#[sqlx::test]
async fn joining_a_search_result_replaces_only_its_card_and_archived_trips_can_be_removed(
    pool: PgPool,
) {
    let (creator, _) = session(&pool, "creator@duke.edu").await;
    let (_, cookie) = session(&pool, "traveler@duke.edu").await;
    let id = data::create(&pool, creator, &input())
        .await
        .unwrap()
        .flight_id;
    let service = app(pool.clone());
    let mut response = TestClient::post(format!("http://localhost:8000/flights/{id}/join"))
        .add_header("origin", "http://localhost:8000", true)
        .add_header("cookie", &cookie, true)
        .add_header("hx-request", "true", true)
        .add_header("hx-target", format!("article#flight-card-{id}"), true)
        .send(&service)
        .await;
    assert_eq!(response.status_code, Some(StatusCode::OK));
    let html = response.take_string().await.unwrap();
    assert!(html.starts_with(&format!("<article id=\"flight-card-{id}\"")));
    assert!(html.contains("Added to your flights"));
    assert!(html.contains("2 from your school"));
    assert!(!html.contains("This is my flight"));
    assert!(!html.contains("id=\"flight-summary\""));
    sqlx::query("UPDATE flights SET deleted_at=now(),revision=revision+1 WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let mut mine_page = get(&service, "/flights/mine", Some(&cookie), None).await;
    assert!(
        mine_page
            .take_string()
            .await
            .unwrap()
            .contains("Removed by admin")
    );
    let mut details = get(&service, &format!("/flights/{id}"), Some(&cookie), None).await;
    assert_eq!(details.status_code, Some(StatusCode::OK));
    let html = details.take_string().await.unwrap();
    assert!(html.contains("Remove from my flights"));
    assert!(!html.contains("Confirm these details"));
    assert!(!html.contains("Something looks wrong"));
    let response = post(
        &service,
        &format!("/flights/{id}/leave"),
        &cookie,
        false,
        &[],
    )
    .await;
    assert_eq!(response.headers()["location"], "/flights/mine");
    assert_eq!(
        get(&service, &format!("/flights/{id}"), Some(&cookie), None)
            .await
            .status_code,
        Some(StatusCode::NOT_FOUND)
    );
}

#[sqlx::test]
async fn inline_creation_keeps_one_navigation_and_preserves_its_review_context(pool: PgPool) {
    let (_, cookie) = session(&pool, "student@duke.edu").await;
    let service = app(pool);
    let mut response = get(
        &service,
        "/flights/new?number=AA123&date=2027-03-20",
        Some(&cookie),
        Some("div#flight-create-slot"),
    )
    .await;
    let html = response.take_string().await.unwrap();
    assert!(html.contains("id=\"flight-create-panel\""));
    assert!(html.contains("<h2"));
    assert!(!html.contains("<h1"));
    assert!(!html.contains("aria-label=\"Flights\""));
    assert!(html.contains("/flights/new/review?inline=true"));
    let mut response = post(
        &service,
        "/flights/new/review?inline=true",
        &cookie,
        true,
        &form(),
    )
    .await;
    let html = response.take_string().await.unwrap();
    assert!(!html.contains("<h1"));
    assert!(html.contains("/flights/new/edit?inline=true"));
    assert!(html.contains("/flights/new/create?inline=true"));
    let mut response = post(
        &service,
        "/flights/new/edit?inline=true",
        &cookie,
        true,
        &form(),
    )
    .await;
    let html = response.take_string().await.unwrap();
    assert!(html.contains("value=\"15:30\""));
    assert!(!html.contains("<h1"));
}
