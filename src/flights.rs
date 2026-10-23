pub(crate) mod catalog;
pub(crate) mod data;
mod views;

use salvo::prelude::*;
use sqlx::PgPool;

use crate::{
    auth,
    templates::{HtmlTemplate, PageView, htmx_target_id, is_htmx},
};
use data::DomainError;
use views::{
    CardResponse, DetailPage, FlightForm, MinePage, NewPage, Results, SearchPage, Summary,
};

pub fn routes() -> Router {
    Router::with_path("flights")
        .hoop(auth::protect)
        .hoop(auth::require_verified_user)
        .get(search)
        .push(Router::with_path("mine").get(mine))
        .push(
            Router::with_path("new")
                .get(new_form)
                .push(Router::with_path("review").post(review))
                .push(Router::with_path("edit").post(edit))
                .push(Router::with_path("create").post(create)),
        )
        .push(
            Router::with_path("{id}")
                .get(detail)
                .push(Router::with_path("status").get(status))
                .push(Router::with_path("join").post(join))
                .push(Router::with_path("leave").post(leave))
                .push(Router::with_path("confirm").post(confirm))
                .push(Router::with_path("corrections").post(propose)),
        )
}

fn id(req: &Request) -> Result<i64, StatusError> {
    req.param::<i64>("id")
        .filter(|id| *id > 0)
        .ok_or_else(StatusError::not_found)
}

fn page(req: &Request) -> Result<i64, StatusError> {
    match req.query::<String>("page") {
        None => Ok(1),
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|p| (1..=10_000).contains(p))
            .ok_or_else(StatusError::bad_request),
    }
}

fn context(depot: &Depot) -> (&PgPool, i64) {
    (
        depot.get_typed::<PgPool>().unwrap(),
        depot.get_typed::<auth::CurrentUser>().unwrap().id,
    )
}

fn error_message(error: &DomainError) -> (StatusCode, String) {
    match error {
        DomainError::Database(error) => {
            tracing::error!(%error, "Flight operation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "We couldn’t save or load this flight right now. Please try again.".into(),
            )
        }
        DomainError::NotFound => (
            StatusCode::NOT_FOUND,
            "This flight is no longer available. Find another flight to continue.".into(),
        ),
        DomainError::Forbidden => (
            StatusCode::FORBIDDEN,
            "Add this flight to your flights before suggesting or confirming its details.".into(),
        ),
        DomainError::Conflict(message) => (StatusCode::CONFLICT, (*message).into()),
        DomainError::Invalid(message) => (StatusCode::BAD_REQUEST, (*message).into()),
    }
}

fn internal(error: DomainError) -> StatusError {
    match error {
        DomainError::NotFound => StatusError::not_found(),
        DomainError::Forbidden => StatusError::forbidden(),
        _ => {
            tracing::error!(%error, "Flight query failed");
            StatusError::internal_server_error()
        }
    }
}

fn redirect(req: &Request, res: &mut Response, location: String) {
    if is_htmx(req) {
        res.headers_mut()
            .insert("hx-redirect", location.parse().unwrap());
        res.status_code(StatusCode::OK);
    } else {
        res.render(Redirect::other(location));
    }
}

#[handler]
async fn search(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let number = req
        .query::<String>("number")
        .unwrap_or_default()
        .trim()
        .to_uppercase();
    let date = req.query::<String>("date").unwrap_or_default();
    let page = page(req)?;
    let searched = !number.is_empty() && !date.is_empty();
    let (pool, user_id) = context(depot);
    let mut results = Results {
        flights: Vec::new(),
        searched,
        error: String::new(),
        number: number.clone(),
        date: date.clone(),
        page,
        has_next: false,
    };
    if searched {
        match data::search(pool, user_id, &number, &date, page).await {
            Ok(mut flights) => {
                results.has_next = flights.len() > 25;
                flights.truncate(25);
                results.flights = flights;
            }
            Err(error) => {
                let (code, message) = error_message(&error);
                res.status_code(code);
                results.error = message;
            }
        }
    }
    if is_htmx(req) && htmx_target_id(req) == Some("flight-results") {
        res.render(HtmlTemplate(results));
    } else {
        SearchPage {
            number,
            date,
            results,
        }
        .show(req, res);
    }
    Ok(())
}

#[handler]
async fn mine(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let page = page(req)?;
    let (pool, user_id) = context(depot);
    let mut flights = data::my_flights(pool, user_id, page)
        .await
        .map_err(internal)?;
    let has_next = flights.len() > 25;
    flights.truncate(25);
    MinePage {
        flights,
        page,
        has_next,
    }
    .show(req, res);
    Ok(())
}

#[handler]
async fn new_form(req: &mut Request, res: &mut Response) {
    let number = req
        .query::<String>("number")
        .unwrap_or_default()
        .replace(' ', "")
        .to_uppercase();
    let airline = catalog::AIRLINES
        .iter()
        .find(|a| number.starts_with(a.code));
    let form = FlightForm {
        airline: airline.map_or_else(String::new, |a| a.code.into()),
        number: airline.map_or(number.clone(), |a| number[a.code.len()..].into()),
        departure_date: req.query::<String>("date").unwrap_or_default(),
        ..Default::default()
    };
    NewPage::new(form, String::new(), false)
        .for_request(req)
        .show(req, res);
}

async fn read_form(req: &mut Request) -> FlightForm {
    FlightForm {
        airline: req
            .form::<String>("airline")
            .await
            .unwrap_or_default()
            .trim()
            .to_uppercase(),
        number: req
            .form::<String>("number")
            .await
            .unwrap_or_default()
            .trim()
            .to_uppercase(),
        departure_date: req
            .form::<String>("departure_date")
            .await
            .unwrap_or_default(),
        departure_time: req
            .form::<String>("departure_time")
            .await
            .unwrap_or_default(),
        origin: req
            .form::<String>("origin")
            .await
            .unwrap_or_default()
            .trim()
            .to_uppercase(),
        destination: req
            .form::<String>("destination")
            .await
            .unwrap_or_default()
            .trim()
            .to_uppercase(),
    }
}

#[handler]
async fn edit(req: &mut Request, res: &mut Response) {
    let form = read_form(req).await;
    NewPage::new(form, String::new(), false)
        .for_request(req)
        .show(req, res);
}

#[handler]
async fn review(req: &mut Request, res: &mut Response) {
    let form = read_form(req).await;
    match data::validate_input(&form.input()) {
        Ok(()) => NewPage::new(form, String::new(), true)
            .for_request(req)
            .show(req, res),
        Err(error) => {
            let (code, message) = error_message(&error);
            res.status_code(code);
            NewPage::new(form, message, false)
                .for_request(req)
                .show(req, res);
        }
    }
}

#[handler]
async fn create(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let form = read_form(req).await;
    let (pool, user_id) = context(depot);
    match data::create(pool, user_id, &form.input()).await {
        Ok(outcome) => redirect(
            req,
            res,
            format!(
                "/flights/{}?notice={}",
                outcome.flight_id,
                if outcome.created {
                    "created"
                } else {
                    "existing"
                }
            ),
        ),
        Err(error) => {
            let (code, message) = error_message(&error);
            res.status_code(code);
            NewPage::new(form, message, false)
                .for_request(req)
                .show(req, res);
        }
    }
}

async fn summary(depot: &Depot, id: i64, notice: String) -> Result<Summary, StatusError> {
    let (pool, user_id) = context(depot);
    let flight = data::flight(pool, user_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(StatusError::not_found)?;
    let corrections = data::corrections(pool, id).await.map_err(internal)?;
    Ok(Summary {
        flight,
        corrections,
        error: String::new(),
        notice,
    })
}

fn detail_page(summary: Summary) -> DetailPage {
    DetailPage {
        summary,
        error: String::new(),
        correction_field: "departure_time".into(),
        correction_value: String::new(),
        correction_reason: String::new(),
        correction_open: false,
    }
}

#[handler]
async fn detail(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let notice = match req.query::<String>("notice").as_deref() {
        Some("created") => "Your flight is added. Other travelers can now find it.",
        Some("existing") => {
            "This flight already exists. Check its details, then join it or suggest a correction."
        }
        Some("suggested") => {
            "Your correction was sent for review. The listed details stay in place until it is reviewed."
        }
        _ => "",
    };
    let summary = summary(depot, id(req)?, notice.into()).await?;
    if let Some(merged) = summary.flight.merged_into {
        redirect(req, res, format!("/flights/{merged}"));
        return Ok(());
    }
    if summary.flight.deleted && !summary.flight.joined {
        return Err(StatusError::not_found());
    }
    detail_page(summary).show(req, res);
    Ok(())
}

#[handler]
async fn status(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = id(req)?;
    if !is_htmx(req) {
        redirect(req, res, format!("/flights/{id}"));
        return Ok(());
    }
    let mut summary = summary(depot, id, String::new()).await?;
    if let Some(merged) = summary.flight.merged_into {
        redirect(req, res, format!("/flights/{merged}"));
        return Ok(());
    }
    if summary.flight.deleted {
        redirect(req, res, format!("/flights/{id}"));
        return Ok(());
    }
    if summary.flight.pending_corrections == 0 {
        summary.notice =
            "The review has been updated. Check the current flight details below.".into();
    }
    res.render(HtmlTemplate(summary));
    Ok(())
}

async fn finish_action(
    req: &Request,
    depot: &Depot,
    res: &mut Response,
    id: i64,
    result: Result<(), DomainError>,
    notice: &str,
) -> Result<(), StatusError> {
    let card_target = format!("flight-card-{id}");
    if is_htmx(req) && htmx_target_id(req) == Some(card_target.as_str()) {
        let (pool, user_id) = context(depot);
        let flight = data::flight(pool, user_id, id)
            .await
            .map_err(internal)?
            .ok_or_else(StatusError::not_found)?;
        let mut card = CardResponse {
            flight,
            error: String::new(),
            notice: notice.into(),
        };
        if let Err(error) = result {
            let (code, message) = error_message(&error);
            res.status_code(code);
            card.error = message;
            card.notice.clear();
        }
        res.render(HtmlTemplate(card));
        return Ok(());
    }
    match result {
        Ok(()) if !is_htmx(req) => redirect(req, res, format!("/flights/{id}")),
        result => {
            let mut summary = summary(depot, id, notice.into()).await?;
            if let Err(error) = result {
                let (code, message) = error_message(&error);
                res.status_code(code);
                summary.error = message;
                summary.notice.clear();
            }
            if is_htmx(req) {
                res.render(HtmlTemplate(summary));
            } else {
                detail_page(summary).show(req, res);
            }
        }
    }
    Ok(())
}

#[handler]
async fn join(req: &mut Request, depot: &mut Depot, res: &mut Response) -> Result<(), StatusError> {
    let id = id(req)?;
    let (pool, user_id) = context(depot);
    let result = data::join(pool, user_id, id).await;
    finish_action(req, depot, res, id, result, "Added to your flights.").await
}

#[handler]
async fn leave(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = id(req)?;
    let (pool, user_id) = context(depot);
    let removed = data::flight(pool, user_id, id)
        .await
        .map_err(internal)?
        .is_some_and(|flight| flight.deleted);
    let result = data::leave(pool, user_id, id).await;
    if removed && result.is_ok() {
        redirect(req, res, "/flights/mine".into());
        return Ok(());
    }
    finish_action(req, depot, res, id, result, "Removed from your flights.").await
}

#[handler]
async fn confirm(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = id(req)?;
    let revision = req.form::<i64>("revision").await.unwrap_or(0);
    let (pool, user_id) = context(depot);
    let result = data::confirm(pool, user_id, id, revision).await;
    finish_action(
        req,
        depot,
        res,
        id,
        result,
        "You confirmed the current flight details.",
    )
    .await
}

#[handler]
async fn propose(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> Result<(), StatusError> {
    let id = id(req)?;
    let revision = req.form::<i64>("revision").await.unwrap_or(0);
    let field = req.form::<String>("field").await.unwrap_or_default();
    let value = req.form::<String>("value").await.unwrap_or_default();
    let reason = req.form::<String>("reason").await.unwrap_or_default();
    let (pool, user_id) = context(depot);
    match data::propose(pool, user_id, id, revision, &field, &value, &reason).await {
        Ok(()) if is_htmx(req) => {
            detail_page(summary(depot, id,
                "Your correction was sent for review. The listed details stay in place until it is reviewed.".into()
            ).await?).show(req, res);
        }
        Ok(()) => redirect(req, res, format!("/flights/{id}?notice=suggested")),
        Err(error) => {
            let (code, message) = error_message(&error);
            let mut page = detail_page(summary(depot, id, String::new()).await?);
            page.error = message;
            page.correction_field = field;
            page.correction_value = value;
            page.correction_reason = reason;
            page.correction_open = true;
            res.status_code(code);
            page.show(req, res);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
