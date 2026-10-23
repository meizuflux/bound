use askama::Template;
use salvo::prelude::*;

/// Renders an Askama template as an HTML response for Salvo.
pub struct HtmlTemplate<T>(pub T);

impl<T: Template> Scribe for HtmlTemplate<T> {
    fn render(self, res: &mut Response) {
        match self.0.render() {
            Ok(html) => res.render(Text::Html(html)),
            Err(error) => {
                tracing::error!(%error, "Failed to render template");
                res.render(StatusError::internal_server_error());
            }
        }
    }
}

pub(crate) fn is_htmx(req: &Request) -> bool {
    req.headers()
        .get("hx-request")
        .is_some_and(|value| value == "true")
}

/// HTMX 4 identifies a target as `tag#id`, rather than just its element ID.
pub(crate) fn htmx_target_id(req: &Request) -> Option<&str> {
    let target = req.headers().get("hx-target")?.to_str().ok()?;
    Some(target.rsplit_once('#').map_or(target, |(_, id)| id))
}

// Only typed Askama templates (with their own escaping) enter this shell.
#[derive(Template)]
#[template(path = "page.html")]
struct Page<T: Template> {
    title: &'static str,
    content: T,
}

// HTMX extracts this title before swapping the typed panel into the page.
#[derive(Template)]
#[template(
    source = "<title>{{ title }} — Bound</title>{{ content|safe }}",
    ext = "html"
)]
struct Panel<T: Template> {
    title: &'static str,
    content: T,
}

/// Render a panel directly for HTMX, or inside the shared layout for navigation.
pub(crate) trait PageView: Template {
    fn title(&self) -> &'static str;

    fn show(self, req: &Request, res: &mut Response)
    where
        Self: Sized,
    {
        let full_page = !is_htmx(req)
            || req
                .headers()
                .get("hx-request-type")
                .is_some_and(|value| value == "full")
            || req
                .headers()
                .get("hx-history-restore-request")
                .is_some_and(|value| value == "true");
        if !full_page {
            res.render(HtmlTemplate(Panel {
                title: self.title(),
                content: self,
            }));
        } else {
            res.render(HtmlTemplate(Page {
                title: self.title(),
                content: self,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;

    use salvo::http::ResBody;

    use super::*;

    #[test]
    fn history_restoration_keeps_the_page_shell_and_partial_navigation_updates_title() {
        #[derive(Template)]
        #[template(source = "<section id=\"test-panel\">Content</section>", ext = "html")]
        struct TestPage;
        impl PageView for TestPage {
            fn title(&self) -> &'static str {
                "Test page"
            }
        }
        for (headers, full) in [
            (vec![], true),
            (vec![("hx-request", "true")], false),
            (
                vec![("hx-request", "true"), ("hx-request-type", "full")],
                true,
            ),
            (
                vec![
                    ("hx-request", "true"),
                    ("hx-history-restore-request", "true"),
                ],
                true,
            ),
        ] {
            let mut req = Request::new();
            for (name, value) in headers {
                req.headers_mut().insert(name, value.parse().unwrap());
            }
            let mut res = Response::new();
            TestPage.show(&req, &mut res);
            let ResBody::Once(body) = res.take_body() else {
                panic!("Expected HTML")
            };
            let html = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(html.contains("<!doctype html>"), full);
            assert_eq!(html.matches("<header").count(), usize::from(full));
            assert_eq!(html.matches("<footer").count(), usize::from(full));
            assert_eq!(html.matches("id=\"test-panel\"").count(), 1);
            assert!(html.contains("<title>Test page — Bound</title>"));
        }
    }

    #[derive(Template)]
    #[template(source = "Hello, {{ value }}!", ext = "html")]
    struct Greeting<T: fmt::Display> {
        value: T,
    }

    #[test]
    fn renders_html_with_escaped_values() {
        let mut response = Response::new();
        response.render(HtmlTemplate(Greeting { value: "<world>" }));

        assert_eq!(
            response.headers()["content-type"],
            "text/html; charset=utf-8"
        );
        let ResBody::Once(body) = response.take_body() else {
            panic!("Expected rendered HTML");
        };
        assert_eq!(body.as_ref(), b"Hello, &#60;world&#62;!");
    }

    #[test]
    fn rendering_failures_return_500_for_panels_and_wrapped_pages() {
        struct BrokenValue;

        impl fmt::Display for BrokenValue {
            fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
                Err(fmt::Error)
            }
        }

        #[derive(Template)]
        #[template(source = "<section>{{ value }}</section>", ext = "html")]
        struct BrokenView {
            value: BrokenValue,
        }

        impl PageView for BrokenView {
            fn title(&self) -> &'static str {
                "Broken view"
            }
        }

        for htmx in [false, true] {
            let mut req = Request::new();
            if htmx {
                req.headers_mut()
                    .insert("hx-request", "true".parse().unwrap());
            }
            let mut response = Response::new();
            BrokenView { value: BrokenValue }.show(&req, &mut response);
            assert_eq!(
                response.status_code,
                Some(StatusCode::INTERNAL_SERVER_ERROR)
            );
            assert!(response.take_body().is_error());
        }
    }

    #[test]
    fn rendering_failure_returns_internal_server_error() {
        struct BrokenValue;

        impl fmt::Display for BrokenValue {
            fn fmt(&self, _formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                Err(fmt::Error)
            }
        }

        let mut response = Response::new();
        response.render(HtmlTemplate(Greeting { value: BrokenValue }));

        assert_eq!(
            response.status_code,
            Some(StatusCode::INTERNAL_SERVER_ERROR)
        );
        assert!(response.take_body().is_error());
    }
}
