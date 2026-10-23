mod data;
mod flight_handlers;
mod mutations;
mod user_handlers;
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
        .push(
            Router::with_path("users")
                .get(list_users)
                .post(user_handlers::create),
        )
        .push(Router::with_path("users/new").get(user_handlers::new))
        .push(
            Router::with_path("users/{id}/delete")
                .get(user_handlers::delete_form)
                .post(user_handlers::delete),
        )
        .push(Router::with_path("users/{id}/update").post(user_handlers::update))
        .push(
            Router::with_path("flights")
                .get(flight_handlers::list)
                .post(flight_handlers::create),
        )
        .push(Router::with_path("flights/new").get(flight_handlers::new))
        .push(
            Router::with_path("flights/{id}")
                .get(flight_handlers::show)
                .post(flight_handlers::update),
        )
        .push(Router::with_path("flights/{id}/aliases").post(flight_handlers::alias))
        .push(Router::with_path("flights/{id}/status").post(flight_handlers::status))
        .push(
            Router::with_path("flights/{id}/merge")
                .get(flight_handlers::merge_form)
                .post(flight_handlers::merge),
        )
        .push(Router::with_path("corrections").get(flight_handlers::list_corrections))
        .push(Router::with_path("corrections/{id}/review").post(flight_handlers::review))
        .push(Router::with_path("users/{id}").get(show_user))
        .push(Router::with_path("activity").get(list_activity))
}

pub(super) fn database_error(error: sqlx::Error) -> StatusError {
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
        message: req.query::<String>("message").unwrap_or_default(),
        error: String::new(),
        own_account: depot.get_typed::<auth::CurrentUser>().unwrap().id == id,
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

fn id(req: &Request) -> Result<i64, StatusError> {
    req.param::<i64>("id")
        .filter(|id| *id > 0)
        .ok_or_else(StatusError::not_found)
}
fn redirect(req: &Request, res: &mut Response, path: String) {
    if crate::templates::is_htmx(req) {
        // All destinations are generated internally from IDs and fixed messages.
        // HTMX follows this with a GET and swaps the admin panel without reloading the shell.
        let location =
            format!(r##"{{"path":"{path}","target":"#admin-panel","swap":"outerHTML"}}"##);
        res.headers_mut()
            .insert("hx-location", location.parse().unwrap());
        res.status_code(StatusCode::OK);
    } else {
        res.render(Redirect::other(path));
    }
}
