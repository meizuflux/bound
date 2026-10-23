//! Typed authentication panels shared by full-page and HTMX responses.

use askama::Template;
use salvo::prelude::*;

use crate::{schools, templates::HtmlTemplate};

use super::is_htmx;

// Only an already-escaped Askama template can be inserted into the page shell.
// The shell's `safe` filter never receives arbitrary user-provided HTML.
#[derive(Template)]
#[template(path = "auth/page.html")]
struct Page<T: Template> {
    title: &'static str,
    content: T,
}

pub(super) trait AuthView: Template {
    fn title(&self) -> &'static str;

    fn show(self, req: &Request, res: &mut Response)
    where
        Self: Sized,
    {
        if is_htmx(req) {
            res.render(HtmlTemplate(self));
        } else {
            res.render(HtmlTemplate(Page {
                title: self.title(),
                content: self,
            }));
        }
    }
}

#[derive(Default)]
pub(super) struct CredentialsForm<'a> {
    pub email: &'a str,
    pub email_error: &'a str,
    pub password_error: &'a str,
    pub error: &'a str,
}

#[derive(Template)]
#[template(path = "partials/auth/signup.html")]
pub(super) struct Signup<'a> {
    form: CredentialsForm<'a>,
    supported_schools: String,
}

impl<'a> Signup<'a> {
    pub fn new(form: CredentialsForm<'a>) -> Self {
        Self {
            form,
            supported_schools: schools::supported_names(),
        }
    }
}

impl AuthView for Signup<'_> {
    fn title(&self) -> &'static str {
        "Create an account"
    }
}

#[derive(Template)]
#[template(path = "partials/auth/signin.html")]
pub(super) struct Signin<'a> {
    pub form: CredentialsForm<'a>,
}

impl AuthView for Signin<'_> {
    fn title(&self) -> &'static str {
        "Sign in"
    }
}

#[derive(Template)]
#[template(path = "partials/auth/account.html")]
pub(super) struct Account<'a> {
    pub email: &'a str,
    pub school: &'a str,
}

impl AuthView for Account<'_> {
    fn title(&self) -> &'static str {
        "Your account"
    }
}

#[derive(Template)]
#[template(path = "partials/auth/verification.html")]
pub(super) struct Verification<'a> {
    pub email: &'a str,
    pub pending: bool,
    pub error: &'a str,
}

impl AuthView for Verification<'_> {
    fn title(&self) -> &'static str {
        if self.pending {
            "Check your school inbox"
        } else {
            "Verify your school email"
        }
    }
}

#[derive(Template)]
#[template(path = "partials/auth/verify.html")]
pub(super) struct Verify<'a> {
    pub token: &'a str,
}

impl AuthView for Verify<'_> {
    fn title(&self) -> &'static str {
        "Verify your school email"
    }
}

#[derive(Template)]
#[template(path = "partials/auth/verification_result.html")]
pub(super) struct VerificationResult {
    pub success: bool,
}

impl AuthView for VerificationResult {
    fn title(&self) -> &'static str {
        if self.success {
            "School email verified"
        } else {
            "Unable to verify this email"
        }
    }
}

#[derive(Template)]
#[template(path = "partials/auth/unsupported.html")]
pub(super) struct Unsupported;

impl AuthView for Unsupported {
    fn title(&self) -> &'static str {
        "School email required"
    }
}

#[cfg(test)]
mod tests;
