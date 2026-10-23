use salvo::http::ResBody;
use salvo::test::{ResponseExt, TestClient};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::*;

fn app(pool: Option<PgPool>) -> Service {
    let mut router = Router::new().hoop(affix_state::inject(auth::Auth::from_env()));
    if let Some(pool) = pool {
        router = router.hoop(affix_state::inject(pool));
    }
    Service::new(router.push(routes()).push(auth::routes()))
}

async fn get(service: &Service, path: &str, htmx: bool, cookie: Option<&str>) -> Response {
    let mut request = TestClient::get(format!("http://localhost:8000{path}"));
    if htmx {
        request = request.add_header("hx-request", "true", true);
    }
    if let Some(cookie) = cookie {
        request = request.add_header("cookie", cookie, true);
    }
    request.send(service).await
}

fn assert_redirect(response: &Response, htmx: bool, path: &str) {
    if htmx {
        assert_eq!(response.status_code, Some(StatusCode::OK));
        assert_eq!(response.headers()["hx-redirect"], path);
    } else {
        assert_eq!(response.status_code, Some(StatusCode::SEE_OTHER));
        assert_eq!(response.headers()["location"], path);
    }
}

#[tokio::test]
async fn standalone_account_redirects_guests_and_is_not_an_auth_route() {
    let service = app(None);
    for htmx in [false, true] {
        let response = get(&service, "/account", htmx, None).await;
        assert_redirect(&response, htmx, "/auth/signin");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["vary"], "HX-Request");
        assert_eq!(
            get(&service, "/auth/account", htmx, None).await.status_code,
            Some(StatusCode::NOT_FOUND)
        );
    }
}

#[sqlx::test]
async fn account_requires_a_verified_supported_email_and_valid_session(pool: PgPool) {
    let service = app(Some(pool.clone()));
    let id: i64 = sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ('student@duke.edu', 'unused') RETURNING id")
        .fetch_one(&pool).await.unwrap();
    let token = "account-test-token";
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, now() + interval '1 day')")
        .bind(hex::encode(Sha256::digest(token.as_bytes()))).bind(id).execute(&pool).await.unwrap();
    let cookie = format!("bound_session={token}");
    for htmx in [false, true] {
        assert_redirect(
            &get(&service, "/account", htmx, Some(&cookie)).await,
            htmx,
            "/auth/verification",
        );
    }
    sqlx::query("UPDATE users SET email_verified_at = now() WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    for htmx in [false, true] {
        let mut response = get(&service, "/account", htmx, Some(&cookie)).await;
        assert_eq!(response.status_code, Some(StatusCode::OK));
        assert_eq!(response.headers()["cache-control"], "no-store");
        let html = response.take_string().await.unwrap();
        assert_eq!(html.contains("<!doctype html>"), !htmx);
        assert_eq!(html.matches("id=\"account-panel\"").count(), 1);
        assert!(!html.contains("id=\"auth-panel\""));
        assert!(html.contains("student@duke.edu · Duke University"));
        assert!(html.contains("hx-post=\"/auth/signout\""));
        assert!(html.contains("hx-target=\"closest section\""));
        assert!(!html.contains("type=\"email\""));
        if !htmx {
            assert!(html.contains("href=\"/account\""));
        }
    }
    sqlx::query("UPDATE users SET email = 'legacy@example.com' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    for htmx in [false, true] {
        assert_redirect(
            &get(&service, "/account", htmx, Some(&cookie)).await,
            htmx,
            "/auth/verification",
        );
    }
    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    for htmx in [false, true] {
        assert_redirect(
            &get(&service, "/account", htmx, Some(&cookie)).await,
            htmx,
            "/auth/signin",
        );
    }
}

#[test]
fn account_values_are_escaped_in_standalone_pages_and_htmx_panels() {
    let payload = "\"><script>alert(1)</script>";
    for htmx in [false, true] {
        let mut request = Request::new();
        if htmx {
            request
                .headers_mut()
                .insert("hx-request", "true".parse().unwrap());
        }
        let mut response = Response::new();
        AccountTemplate {
            email: payload,
            school: payload,
            is_admin: false,
        }
        .show(&request, &mut response);
        let ResBody::Once(body) = response.take_body() else {
            panic!("Expected account HTML");
        };
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(html.contains("<!doctype html>"), !htmx);
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("&amp;#60;script"));
        assert!(html.contains("id=\"account-panel\""));
    }
}
