use salvo::{http::ResBody, prelude::*};

use super::*;

fn request(htmx: bool) -> Request {
    let mut request = Request::new();
    if htmx {
        request
            .headers_mut()
            .insert("hx-request", "true".parse().unwrap());
    }
    request
}

fn render(view: impl PageView, htmx: bool) -> String {
    let title = view.title();
    let mut response = Response::new();
    view.show(&request(htmx), &mut response);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let ResBody::Once(body) = response.take_body() else {
        panic!("Expected rendered authentication HTML");
    };
    let html = String::from_utf8(body.to_vec()).unwrap();
    assert!(html.contains(&format!("<title>{title} — Bound</title>")));
    assert_eq!(html.contains("<!doctype html>"), !htmx);
    assert_eq!(html.matches("id=\"auth-panel\"").count(), 1);
    if htmx {
        assert!(!html.contains("<head>"));
        assert!(!html.contains("<header"));
        assert!(!html.contains("<footer"));
    } else {
        assert!(html.contains(&format!("<title>{title} — Bound</title>")));
        assert_eq!(html.matches("<header").count(), 1);
        assert_eq!(html.matches("<footer").count(), 1);
    }
    html
}

#[test]
fn each_screen_renders_as_a_page_or_a_focused_htmx_panel() {
    for htmx in [false, true] {
        let signup = render(Signup::new(CredentialsForm::default()), htmx);
        assert!(signup.contains("hx-post=\"/auth/signup\""));
        assert!(signup.contains("autocomplete=\"new-password\""));
        assert!(!signup.contains("hx-post=\"/auth/signin\""));
        assert!(!signup.contains("hx-post=\"/auth/resend\""));

        let signin = render(
            Signin {
                form: CredentialsForm::default(),
            },
            htmx,
        );
        assert!(signin.contains("hx-post=\"/auth/signin\""));
        assert!(signin.contains("autocomplete=\"current-password\""));
        assert!(!signin.contains("hx-post=\"/auth/signup\""));
        assert!(!signin.contains("Currently supporting"));

        let pending = render(
            Verification {
                email: "student@duke.edu",
                pending: true,
                error: "",
            },
            htmx,
        );
        assert!(pending.contains("Check your school inbox"));
        assert!(pending.contains("hx-get=\"/auth/verification/status\""));
        assert!(pending.contains("hx-trigger=\"every 2s\""));
        assert!(pending.contains("hx-post=\"/auth/resend\""));
        assert!(pending.contains("hx-post=\"/auth/signout\""));
        assert!(!pending.contains("type=\"email\""));

        let expired = render(
            Verification {
                email: "student@duke.edu",
                pending: false,
                error: "",
            },
            htmx,
        );
        assert!(!expired.contains("every 2s"));
        assert!(expired.contains("Request a new verification link"));
        assert!(expired.contains("hx-post=\"/auth/resend\""));

        let failed = render(
            Verification {
                email: "student@duke.edu",
                pending: false,
                error: "Please retry delivery.",
            },
            htmx,
        );
        assert!(failed.contains("Please retry delivery."));
        assert!(failed.contains("role=\"alert\""));
        assert!(failed.contains("text-red-700"));
        assert!(!failed.contains("every 2s"));

        let verify = render(
            Verify {
                token: "test-token",
            },
            htmx,
        );
        assert!(verify.contains("hx-post=\"/auth/verify\""));
        assert!(verify.contains("hx-trigger=\"load, submit\""));
        assert!(verify.contains("value=\"test-token\""));
        assert!(!verify.contains("hx-post=\"/auth/signout\""));

        let success = render(VerificationResult { success: true }, htmx);
        assert!(success.contains("School email verified"));
        assert!(success.contains("You can now close this page"));
        assert!(!success.contains("Unable to verify"));
        assert!(!success.contains("<form"));

        let invalid = render(VerificationResult { success: false }, htmx);
        assert!(invalid.contains("Unable to verify this email"));
        assert!(invalid.contains("role=\"alert\""));
        assert!(invalid.contains("text-red-700"));
        assert!(invalid.contains("href=\"/auth/verification\""));
        assert!(!invalid.contains("You can now close this page"));

        let unsupported = render(Unsupported, htmx);
        assert!(unsupported.contains("School email required"));
        assert!(unsupported.contains("href=\"/auth/signup\""));
        assert!(unsupported.contains("hx-post=\"/auth/signout\""));
        assert!(!unsupported.contains("hx-post=\"/auth/resend\""));
    }
}

#[test]
fn wrapping_a_panel_never_bypasses_or_duplicates_escaping() {
    let payload = "\"><script>alert(1)</script>";
    for htmx in [false, true] {
        let outputs = [
            render(
                Signup::new(CredentialsForm {
                    email: payload,
                    email_error: payload,
                    password_error: payload,
                    error: payload,
                }),
                htmx,
            ),
            render(
                Signin {
                    form: CredentialsForm {
                        email: payload,
                        error: payload,
                        ..Default::default()
                    },
                },
                htmx,
            ),
            render(
                Verification {
                    email: payload,
                    pending: true,
                    error: "",
                },
                htmx,
            ),
            render(
                Verification {
                    email: "student@duke.edu",
                    pending: false,
                    error: payload,
                },
                htmx,
            ),
            render(Verify { token: payload }, htmx),
        ];
        for html in outputs {
            assert!(!html.contains("<script>alert(1)</script>"));
            assert!(html.contains("&#60;script&#62;"));
            assert!(!html.contains("&amp;#60;script"));
        }
    }
}
