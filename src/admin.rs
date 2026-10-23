mod data;
mod views;

use salvo::prelude::*;
use sqlx::PgPool;

use crate::{auth, templates::PageView};
use views::{ActivityPage, Nav, UserPage, UsersPage};

pub fn routes() -> Router {
    Router::with_path("admin")
        .hoop(auth::protect)
        .hoop(auth::require_admin)
        .get(list_users)
        .push(Router::with_path("users").get(list_users))
        .push(Router::with_path("users/{id}").get(show_user))
        .push(Router::with_path("activity").get(list_activity))
}

fn database_error(error: sqlx::Error) -> StatusError {
    tracing::error!(%error, "Admin query failed");
    StatusError::internal_server_error()
}

#[handler]
async fn list_users(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let mut pagination = views::Pagination::from_request(req)?;
    let mut users = data::users(depot.get_typed::<PgPool>().unwrap(), &pagination)
        .await
        .map_err(database_error)?;
    pagination.finish(&mut users);
    UsersPage {
        nav: Nav::USERS,
        users,
        pagination,
    }
    .show(req, res);
    Ok(())
}

#[handler]
async fn show_user(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = req
        .param::<i64>("id")
        .filter(|id| *id > 0)
        .ok_or_else(StatusError::not_found)?;
    let pool = depot.get_typed::<PgPool>().unwrap();
    let user = data::user(pool, id)
        .await
        .map_err(database_error)?
        .ok_or_else(StatusError::not_found)?;
    let events = data::user_activity(pool, id)
        .await
        .map_err(database_error)?;
    UserPage {
        nav: Nav::USERS,
        user,
        events,
    }
    .show(req, res);
    Ok(())
}

#[handler]
async fn list_activity(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let mut pagination = views::Pagination::from_request(req)?;
    let mut events = data::activity(depot.get_typed::<PgPool>().unwrap(), &pagination)
        .await
        .map_err(database_error)?;
    pagination.finish(&mut events);
    ActivityPage {
        nav: Nav::ACTIVITY,
        events,
        pagination,
    }
    .show(req, res);
    Ok(())
}

#[cfg(test)]
mod tests;
