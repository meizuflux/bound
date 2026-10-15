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

#[cfg(test)]
mod tests {
    use std::fmt;

    use salvo::http::ResBody;

    use super::*;

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
