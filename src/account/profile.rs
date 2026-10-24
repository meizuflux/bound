use askama::Template;
use salvo::prelude::*;
use sqlx::PgPool;

use crate::{
    audit, auth,
    templates::{PageView, is_htmx},
};

pub(super) fn routes() -> Router {
    Router::with_path("profile")
        .get(show)
        .post(save)
        .push(Router::with_path("skip").post(skip))
}

#[derive(sqlx::FromRow)]
struct Profile {
    display_name: String,
    phone_number: String,
    profile_visible: bool,
    share_phone: bool,
    profile_revision: i64,
}

#[derive(Template)]
#[template(path = "partials/account/profile.html")]
struct ProfilePage {
    profile: Profile,
    onboarding: bool,
    error: String,
    name_error: String,
    phone_error: String,
}

impl PageView for ProfilePage {
    fn title(&self) -> &'static str {
        if self.onboarding {
            "Complete your profile"
        } else {
            "Edit your profile"
        }
    }
}

fn internal(error: sqlx::Error) -> StatusError {
    tracing::error!(%error, "Profile query failed");
    StatusError::internal_server_error()
}

fn redirect(req: &Request, res: &mut Response) {
    if is_htmx(req) {
        res.headers_mut()
            .insert("hx-redirect", "/account".parse().unwrap());
        res.status_code(StatusCode::OK);
    } else {
        res.render(Redirect::other("/account"));
    }
}

#[handler]
async fn show(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let user = depot.get_typed::<auth::CurrentUser>().unwrap();
    let profile = sqlx::query_as("SELECT display_name, phone_number, profile_visible, share_phone, profile_revision FROM users WHERE id=$1")
        .bind(user.id).fetch_one(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)?;
    ProfilePage {
        profile,
        onboarding: req.query::<bool>("onboarding") == Some(true),
        error: String::new(),
        name_error: String::new(),
        phone_error: String::new(),
    }
    .show(req, res);
    Ok(())
}

fn validate(page: &mut ProfilePage) -> bool {
    let name = &page.profile.display_name;
    if name.chars().count() > 80 || name.chars().any(char::is_control) {
        page.name_error = "Use a name of up to 80 characters without line breaks.".into();
    }
    let phone = &page.profile.phone_number;
    let digits = phone.chars().filter(char::is_ascii_digit).count();
    if !phone.is_empty()
        && (phone.len() > 32
            || !(7..=15).contains(&digits)
            || !phone
                .chars()
                .enumerate()
                .all(|(i, c)| c.is_ascii_digit() || " ()-.".contains(c) || (i == 0 && c == '+')))
    {
        page.phone_error = "Enter a phone number with 7–15 digits, including your country code, or leave it blank.".into();
    }
    if phone.is_empty() && page.profile.share_phone {
        page.phone_error = "Add a phone number before opting to share it.".into();
    }
    page.name_error.is_empty() && page.phone_error.is_empty()
}

#[handler]
async fn save(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let mut page = ProfilePage {
        profile: Profile {
            display_name: req
                .form::<String>("display_name")
                .await
                .unwrap_or_default()
                .trim()
                .to_owned(),
            phone_number: req
                .form::<String>("phone_number")
                .await
                .unwrap_or_default()
                .trim()
                .to_owned(),
            profile_visible: req.form::<String>("profile_visible").await.as_deref() == Some("on"),
            share_phone: req.form::<String>("share_phone").await.as_deref() == Some("on"),
            profile_revision: req.form::<i64>("revision").await.unwrap_or_default(),
        },
        onboarding: req.form::<bool>("onboarding").await == Some(true),
        error: String::new(),
        name_error: String::new(),
        phone_error: String::new(),
    };
    if !validate(&mut page) {
        res.status_code(StatusCode::BAD_REQUEST);
        page.show(req, res);
        return;
    }
    let user = depot.get_typed::<auth::CurrentUser>().unwrap();
    let pool = depot.get_typed::<PgPool>().unwrap();
    let result = async {
        let mut tx = pool.begin().await?;
        audit::set_actor(&mut tx, user.id).await?;
        let changed = sqlx::query("UPDATE users SET display_name=$2, phone_number=$3, profile_visible=$4, share_phone=$5, profile_onboarding_done=true, profile_revision=profile_revision+1 WHERE id=$1 AND profile_revision=$6 AND email=$7 AND email_verified_at IS NOT NULL AND suspended_at IS NULL")
            .bind(user.id).bind(&page.profile.display_name).bind(&page.profile.phone_number)
            .bind(page.profile.profile_visible).bind(page.profile.share_phone).bind(page.profile.profile_revision)
            .bind(&user.email).execute(&mut *tx).await?.rows_affected();
        if changed == 1 {
            audit::record(&mut tx, user.id, Some(user.id), None, "profile_updated", "Profile and sharing preferences updated.").await?;
        }
        tx.commit().await?;
        Ok::<_, sqlx::Error>(changed)
    }.await;
    match result {
        Ok(1) => {
            redirect(req, res);
            return;
        }
        Ok(_) => {
            res.status_code(StatusCode::CONFLICT);
            page.error = "Your account changed while this page was open. Reload your profile before saving again.".into();
        }
        Err(error) => {
            tracing::error!(%error, "Profile update failed");
            res.status_code(StatusCode::INTERNAL_SERVER_ERROR);
            page.error = "We couldn’t save your profile. Please try again.".into();
        }
    }
    page.show(req, res);
}

#[handler]
async fn skip(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let user = depot.get_typed::<auth::CurrentUser>().unwrap();
    sqlx::query("UPDATE users SET profile_onboarding_done=true WHERE id=$1 AND email=$2 AND email_verified_at IS NOT NULL AND suspended_at IS NULL")
        .bind(user.id).bind(&user.email).execute(depot.get_typed::<PgPool>().unwrap()).await.map_err(internal)?;
    redirect(req, res);
    Ok(())
}

#[cfg(test)]
mod tests;
