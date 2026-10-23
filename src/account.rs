use askama::Template;
use salvo::prelude::*;

use crate::{auth, templates::PageView};

pub fn routes() -> Router {
    Router::with_path("account")
        .hoop(auth::protect)
        .hoop(auth::require_verified_user)
        .get(show)
}

#[derive(Template)]
#[template(path = "partials/account.html")]
struct AccountTemplate<'a> {
    email: &'a str,
    school: &'a str,
}

impl PageView for AccountTemplate<'_> {
    fn title(&self) -> &'static str {
        "Your account"
    }
}

#[handler]
async fn show(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let user = depot
        .get_typed::<auth::CurrentUser>()
        .expect("Verified-user hoop must load the account");
    let school = user
        .verified_school()
        .expect("Verified-user hoop must validate the school");
    AccountTemplate {
        email: &user.email,
        school: school.name,
    }
    .show(req, res);
}

#[cfg(test)]
mod tests;
