use super::{
    id, redirect,
    views::{self, CorrectionsPage, FlightForm, FlightsPage, MergePage, Nav},
};
use crate::{
    auth,
    flights::data::{self, DomainError, FlightInput},
    templates::PageView,
};
use salvo::prelude::*;
use sqlx::PgPool;

fn actor(depot: &Depot) -> i64 {
    depot.get_typed::<auth::CurrentUser>().unwrap().id
}
fn pool(depot: &Depot) -> &PgPool {
    depot.get_typed::<PgPool>().unwrap()
}
fn error(error: DomainError, res: &mut Response) -> Result<String, StatusError> {
    match error {
        DomainError::NotFound => Err(StatusError::not_found()),
        DomainError::Forbidden => Err(StatusError::forbidden()),
        DomainError::Database(error) => {
            tracing::error!(%error,"Admin flight query failed");
            Err(StatusError::internal_server_error())
        }
        DomainError::Invalid(message) => {
            res.status_code(StatusCode::BAD_REQUEST);
            Ok(message.into())
        }
        DomainError::Conflict(message) => {
            res.status_code(StatusCode::CONFLICT);
            Ok(message.into())
        }
    }
}
fn query_error(error: DomainError) -> StatusError {
    match error {
        DomainError::NotFound => StatusError::not_found(),
        DomainError::Forbidden => StatusError::forbidden(),
        DomainError::Invalid(_) => StatusError::bad_request(),
        DomainError::Conflict(_) => StatusError::conflict(),
        DomainError::Database(error) => {
            tracing::error!(%error, "Admin flight query failed");
            StatusError::internal_server_error()
        }
    }
}
async fn input(req: &mut Request) -> FlightInput {
    FlightInput {
        flight_number: req
            .form::<String>("flight_number")
            .await
            .unwrap_or_default(),
        departure_date: req
            .form::<String>("departure_date")
            .await
            .unwrap_or_default(),
        departure_time: req
            .form::<String>("departure_time")
            .await
            .unwrap_or_default(),
        origin: req.form::<String>("origin").await.unwrap_or_default(),
        destination: req.form::<String>("destination").await.unwrap_or_default(),
    }
}
fn form(flight: data::Flight) -> FlightForm {
    FlightForm {
        nav: Nav::FLIGHTS,
        id: flight.id,
        revision: flight.revision,
        input: FlightInput {
            flight_number: flight.flight_number,
            departure_date: flight.departure_date,
            departure_time: flight.departure_time,
            origin: flight.origin,
            destination: flight.destination,
        },
        error: String::new(),
        deleted: flight.deleted,
        attendees: flight.attendees,
        message: String::new(),
        aliases: Vec::new(),
        merged_into: flight.merged_into,
    }
}
#[handler]
pub(super) async fn list(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let mut pagination = views::Pagination::from_request(req)?;
    if pagination.page > 10_000 {
        return Err(StatusError::bad_request());
    }
    let mut flights =
        data::admin_flights(pool(depot), actor(depot), &pagination.q, pagination.page)
            .await
            .map_err(query_error)?;
    pagination.finish(&mut flights);
    FlightsPage {
        nav: Nav::FLIGHTS,
        flights,
        pagination,
    }
    .show(req, res);
    Ok(())
}
#[handler]
pub(super) async fn new(req: &mut Request, res: &mut Response) {
    FlightForm {
        nav: Nav::FLIGHTS,
        id: 0,
        revision: 0,
        input: FlightInput::default(),
        error: String::new(),
        deleted: false,
        attendees: 0,
        message: String::new(),
        aliases: Vec::new(),
        merged_into: None,
    }
    .show(req, res);
}
#[handler]
pub(super) async fn show(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let flight = data::flight(pool(depot), actor(depot), id(req)?)
        .await
        .map_err(query_error)?
        .ok_or_else(StatusError::not_found)?;
    let mut page = form(flight);
    page.aliases = data::aliases(pool(depot), page.id)
        .await
        .map_err(query_error)?;
    page.message = req.query::<String>("message").unwrap_or_default();
    page.show(req, res);
    Ok(())
}
#[handler]
pub(super) async fn create(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let input = input(req).await;
    let distinct = req.form::<String>("distinct").await.as_deref() == Some("on");
    match data::admin_create(pool(depot), actor(depot), &input, distinct).await {
        Ok(outcome) => redirect(
            req,
            res,
            format!(
                "/admin/flights/{}?message={}",
                outcome.flight_id,
                if outcome.created {
                    "Flight%20created."
                } else {
                    "Existing%20flight%20found.%20Review%20before%20editing."
                }
            ),
        ),
        Err(e) => {
            let message = error(e, res)?;
            FlightForm {
                nav: Nav::FLIGHTS,
                id: 0,
                revision: 0,
                input,
                error: message,
                deleted: false,
                attendees: 0,
                message: String::new(),
                aliases: Vec::new(),
                merged_into: None,
            }
            .show(req, res);
        }
    }
    Ok(())
}
#[handler]
pub(super) async fn update(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = id(req)?;
    let revision = req
        .form::<i64>("revision")
        .await
        .ok_or_else(StatusError::bad_request)?;
    let input = input(req).await;
    match data::admin_update(pool(depot), actor(depot), id, revision, &input).await {
        Ok(()) => redirect(
            req,
            res,
            format!("/admin/flights/{id}?message=Flight%20updated."),
        ),
        Err(e) => {
            let message = error(e, res)?;
            let flight = data::flight(pool(depot), actor(depot), id)
                .await
                .map_err(query_error)?
                .ok_or_else(StatusError::not_found)?;
            let mut page = form(flight);
            page.aliases = data::aliases(pool(depot), id).await.map_err(query_error)?;
            page.input = input;
            page.revision = revision;
            page.error = message;
            page.show(req, res);
        }
    }
    Ok(())
}
#[handler]
pub(super) async fn status(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = id(req)?;
    let revision = req
        .form::<i64>("revision")
        .await
        .ok_or_else(StatusError::bad_request)?;
    let deleted = req.form::<String>("deleted").await.as_deref() == Some("true");
    let confirmed = req.form::<String>("confirm").await.as_deref() == Some("on");
    let result = if confirmed {
        data::admin_set_deleted(pool(depot), actor(depot), id, revision, deleted).await
    } else {
        Err(DomainError::Invalid(
            "Confirm the change before continuing.",
        ))
    };
    match result {
        Ok(()) => redirect(
            req,
            res,
            format!("/admin/flights/{id}?message=Flight%20availability%20updated."),
        ),
        Err(e) => {
            let message = error(e, res)?;
            let flight = data::flight(pool(depot), actor(depot), id)
                .await
                .map_err(query_error)?
                .ok_or_else(StatusError::not_found)?;
            let mut page = form(flight);
            page.aliases = data::aliases(pool(depot), id).await.map_err(query_error)?;
            page.error = message;
            page.show(req, res);
        }
    }
    Ok(())
}
#[handler]
pub(super) async fn list_corrections(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let mut pagination = views::Pagination::from_request(req)?;
    if pagination.page > 10_000 {
        return Err(StatusError::bad_request());
    }
    let mut corrections = data::pending_corrections(pool(depot), actor(depot), pagination.page)
        .await
        .map_err(query_error)?;
    pagination.finish(&mut corrections);
    CorrectionsPage {
        nav: Nav::CORRECTIONS,
        corrections,
        pagination,
        error: String::new(),
    }
    .show(req, res);
    Ok(())
}
#[handler]
pub(super) async fn review(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let approve = match req.form::<String>("decision").await.as_deref() {
        Some("approve") => true,
        Some("reject") => false,
        _ => return Err(StatusError::bad_request()),
    };
    match data::review(pool(depot), actor(depot), id(req)?, approve).await {
        Ok(()) => redirect(req, res, "/admin/corrections".into()),
        Err(e) => {
            let message = error(e, res)?;
            let pagination = views::Pagination {
                q: String::new(),
                page: 1,
                has_next: false,
            };
            let corrections = data::pending_corrections(pool(depot), actor(depot), 1)
                .await
                .map_err(query_error)?;
            CorrectionsPage {
                nav: Nav::CORRECTIONS,
                corrections,
                pagination,
                error: message,
            }
            .show(req, res);
        }
    }
    Ok(())
}
#[handler]
pub(super) async fn merge_form(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let source = data::flight(pool(depot), actor(depot), id(req)?)
        .await
        .map_err(query_error)?
        .ok_or_else(StatusError::not_found)?;
    let target = if let Some(target) = req.query::<i64>("target") {
        data::flight(pool(depot), actor(depot), target)
            .await
            .map_err(query_error)?
    } else {
        None
    };
    let message = if req.query::<String>("target").is_some() && target.is_none() {
        "No flight has that ID. Choose an existing destination flight.".into()
    } else {
        String::new()
    };
    MergePage {
        nav: Nav::FLIGHTS,
        source,
        target,
        error: message,
    }
    .show(req, res);
    Ok(())
}
#[handler]
pub(super) async fn merge(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let source_id = id(req)?;
    let target_id = req
        .form::<i64>("target_id")
        .await
        .ok_or_else(StatusError::bad_request)?;
    let source_revision = req
        .form::<i64>("source_revision")
        .await
        .ok_or_else(StatusError::bad_request)?;
    let target_revision = req
        .form::<i64>("target_revision")
        .await
        .ok_or_else(StatusError::bad_request)?;
    let confirmed = req.form::<String>("confirm").await.as_deref() == Some("on");
    let result = if confirmed {
        data::admin_merge(
            pool(depot),
            actor(depot),
            source_id,
            target_id,
            source_revision,
            target_revision,
        )
        .await
    } else {
        Err(DomainError::Invalid("Confirm the merge before continuing."))
    };
    match result {
        Ok(()) => redirect(
            req,
            res,
            format!("/admin/flights/{target_id}?message=Flights%20merged."),
        ),
        Err(e) => {
            let message = error(e, res)?;
            let source = data::flight(pool(depot), actor(depot), source_id)
                .await
                .map_err(query_error)?
                .ok_or_else(StatusError::not_found)?;
            let target = data::flight(pool(depot), actor(depot), target_id)
                .await
                .map_err(query_error)?;
            MergePage {
                nav: Nav::FLIGHTS,
                source,
                target,
                error: message,
            }
            .show(req, res);
        }
    }
    Ok(())
}

#[handler]
pub(super) async fn alias(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = id(req)?;
    let number = req.form::<String>("number").await.unwrap_or_default();
    let remove = req.form::<String>("remove").await.as_deref() == Some("true");
    let result = if remove {
        data::admin_remove_alias(pool(depot), actor(depot), id, &number).await
    } else {
        data::admin_add_alias(pool(depot), actor(depot), id, &number).await
    };
    match result {
        Ok(()) => redirect(
            req,
            res,
            format!("/admin/flights/{id}?message=Alternate%20flight%20numbers%20updated."),
        ),
        Err(e) => {
            let message = error(e, res)?;
            let flight = data::flight(pool(depot), actor(depot), id)
                .await
                .map_err(query_error)?
                .ok_or_else(StatusError::not_found)?;
            let mut page = form(flight);
            page.aliases = data::aliases(pool(depot), id).await.map_err(query_error)?;
            page.error = message;
            page.show(req, res);
        }
    }
    Ok(())
}
