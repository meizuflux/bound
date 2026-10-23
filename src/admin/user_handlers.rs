use super::{
    data, database_error, id,
    mutations::{self, WriteError},
    redirect,
    views::{Nav, UserDelete, UserForm, UserPage},
};
use crate::{auth, schools, templates::PageView};
use salvo::prelude::*;
use sqlx::PgPool;

fn write_error(error: WriteError) -> Result<&'static str, StatusError> {
    match error {
        WriteError::Conflict(message) => Ok(message),
        WriteError::Missing => Err(StatusError::not_found()),
        WriteError::Database(error) => {
            tracing::error!(%error,"Admin write failed");
            Err(StatusError::internal_server_error())
        }
    }
}
#[handler]
pub(super) async fn new(req: &mut Request, res: &mut Response) {
    UserForm {
        nav: Nav::USERS,
        email: String::new(),
        is_admin: false,
        error: String::new(),
    }
    .show(req, res);
}
#[handler]
pub(super) async fn create(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let email = req.form::<String>("email").await.unwrap_or_default();
    let password = req.form::<String>("password").await.unwrap_or_default();
    let is_admin = req.form::<String>("is_admin").await.as_deref() == Some("on");
    let normalized =
        auth::normalize_email(&email).filter(|email| schools::from_email(email).is_some());
    let error = if normalized.is_none() {
        Some("Enter a supported school email address (duke.edu).")
    } else if !(12..=128).contains(&password.len()) {
        Some("Use an initial password between 12 and 128 characters.")
    } else {
        None
    };
    let mut code = StatusCode::BAD_REQUEST;
    let error = if let Some(error) = error {
        error
    } else {
        let pool = depot.get_typed::<PgPool>().unwrap();
        let actor = depot.get_typed::<auth::CurrentUser>().unwrap().id;
        let password_hash = tokio::task::spawn_blocking(move || auth::hash_password(&password))
            .await
            .map_err(|_| StatusError::internal_server_error())?;
        match mutations::create(
            pool,
            actor,
            normalized.as_deref().unwrap(),
            &password_hash,
            is_admin,
        )
        .await
        {
            Ok(id) => {
                let message = match auth::send_account_verification(
                    pool,
                    depot.get_typed::<auth::Auth>().unwrap(),
                    id,
                    normalized.as_deref().unwrap(),
                )
                .await
                {
                    Ok(()) => "Account created. Verification email sent.",
                    Err(error) => {
                        tracing::error!(%error,"Admin-created user verification delivery failed");
                        "Account created. Email delivery failed; the user can sign in and resend verification."
                    }
                };
                redirect(
                    req,
                    res,
                    format!("/admin/users/{id}?message={}", message.replace(' ', "%20")),
                );
                return Ok(());
            }
            Err(error) => {
                code = StatusCode::CONFLICT;
                write_error(error)?
            }
        }
    };
    res.status_code(code);
    UserForm {
        nav: Nav::USERS,
        email,
        is_admin,
        error: error.into(),
    }
    .show(req, res);
    Ok(())
}
#[handler]
pub(super) async fn update(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let target = id(req)?;
    let actor = depot.get_typed::<auth::CurrentUser>().unwrap().id;
    let version = req
        .form::<i64>("version")
        .await
        .ok_or_else(StatusError::bad_request)?;
    let admin = req.form::<String>("is_admin").await.as_deref() == Some("on");
    let suspended = req.form::<String>("suspended").await.as_deref() == Some("on");
    let revoke = req.form::<String>("revoke_sessions").await.as_deref() == Some("on");
    let pool = depot.get_typed::<PgPool>().unwrap();
    match mutations::update(pool, actor, target, version, admin, suspended, revoke).await {
        Ok(()) => redirect(
            req,
            res,
            format!("/admin/users/{target}?message=Account%20updated."),
        ),
        Err(error) => {
            let message = write_error(error)?;
            res.status_code(StatusCode::CONFLICT);
            UserPage {
                nav: Nav::USERS,
                user: data::user(pool, target)
                    .await
                    .map_err(database_error)?
                    .ok_or_else(StatusError::not_found)?,
                events: data::user_activity(pool, target)
                    .await
                    .map_err(database_error)?,
                message: String::new(),
                error: message.into(),
                own_account: actor == target,
            }
            .show(req, res);
        }
    }
    Ok(())
}
#[handler]
pub(super) async fn delete_form(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let target = id(req)?;
    let user = data::user(depot.get_typed::<PgPool>().unwrap(), target)
        .await
        .map_err(database_error)?
        .ok_or_else(StatusError::not_found)?;
    UserDelete {
        nav: Nav::USERS,
        user,
        error: String::new(),
    }
    .show(req, res);
    Ok(())
}
#[handler]
pub(super) async fn delete(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let target = id(req)?;
    let version = req
        .form::<i64>("version")
        .await
        .ok_or_else(StatusError::bad_request)?;
    let pool = depot.get_typed::<PgPool>().unwrap();
    let user = data::user(pool, target)
        .await
        .map_err(database_error)?
        .ok_or_else(StatusError::not_found)?;
    let confirm = req
        .form::<String>("confirm_email")
        .await
        .unwrap_or_default();
    let error = if confirm != user.email {
        "Enter the account email to confirm deletion."
    } else {
        match mutations::delete(
            pool,
            depot.get_typed::<auth::CurrentUser>().unwrap().id,
            target,
            version,
        )
        .await
        {
            Ok(()) => {
                redirect(req, res, "/admin/users".into());
                return Ok(());
            }
            Err(error) => write_error(error)?,
        }
    };
    res.status_code(StatusCode::CONFLICT);
    UserDelete {
        nav: Nav::USERS,
        user,
        error: error.into(),
    }
    .show(req, res);
    Ok(())
}
