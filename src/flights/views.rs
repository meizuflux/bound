use askama::Template;
use salvo::prelude::Request;

use super::{catalog, data};
use crate::templates::PageView;

#[derive(Default, Clone)]
pub(super) struct FlightForm {
    pub airline: String,
    pub number: String,
    pub departure_date: String,
    pub departure_time: String,
    pub origin: String,
    pub destination: String,
}

impl FlightForm {
    pub fn input(&self) -> data::FlightInput {
        data::FlightInput {
            flight_number: format!("{}{}", self.airline, self.number),
            departure_date: self.departure_date.clone(),
            departure_time: self.departure_time.clone(),
            origin: self.origin.clone(),
            destination: self.destination.clone(),
        }
    }
    pub fn timezone(&self) -> &str {
        catalog::airport(&self.origin).map_or("the origin airport’s local time", |a| a.timezone)
    }
}

#[derive(Template)]
#[template(path = "partials/flights/search.html")]
pub(super) struct SearchPage {
    pub number: String,
    pub date: String,
    pub results: Results,
}
impl PageView for SearchPage {
    fn title(&self) -> &'static str {
        "Find a flight"
    }
}

#[derive(Template)]
#[template(path = "partials/flights/results.html")]
pub(super) struct Results {
    pub flights: Vec<data::Flight>,
    pub searched: bool,
    pub error: String,
    pub number: String,
    pub date: String,
    pub page: i64,
    pub has_next: bool,
}

#[derive(Template)]
#[template(path = "partials/flights/new.html")]
pub(super) struct NewPage {
    pub form: FlightForm,
    pub error: String,
    pub review: bool,
    pub inline: bool,
    pub airlines: &'static [catalog::Airline],
    pub airports: &'static [catalog::Airport],
}
impl NewPage {
    pub fn new(form: FlightForm, error: String, review: bool) -> Self {
        Self {
            form,
            error,
            review,
            inline: false,
            airlines: catalog::AIRLINES,
            airports: catalog::AIRPORTS,
        }
    }
    pub fn for_request(mut self, req: &Request) -> Self {
        self.inline = crate::templates::is_htmx(req)
            && (crate::templates::htmx_target_id(req) == Some("flight-create-slot")
                || req.query::<bool>("inline") == Some(true));
        self
    }
}
impl PageView for NewPage {
    fn title(&self) -> &'static str {
        "Add a flight"
    }
}

#[derive(Template)]
#[template(path = "partials/flights/mine.html")]
pub(super) struct MinePage {
    pub flights: Vec<data::Flight>,
    pub page: i64,
    pub has_next: bool,
}
impl PageView for MinePage {
    fn title(&self) -> &'static str {
        "My flights"
    }
}

#[derive(Template)]
#[template(path = "partials/flights/detail.html")]
pub(super) struct DetailPage {
    pub summary: Summary,
    pub error: String,
    pub correction_field: String,
    pub correction_value: String,
    pub correction_reason: String,
    pub correction_open: bool,
}
impl PageView for DetailPage {
    fn title(&self) -> &'static str {
        "Flight details"
    }
}

#[derive(Template)]
#[template(path = "partials/flights/summary.html")]
pub(super) struct Summary {
    pub flight: data::Flight,
    pub corrections: Vec<data::Correction>,
    pub error: String,
    pub notice: String,
}

impl data::Correction {
    pub(crate) fn field_label(&self) -> &str {
        match self.field.as_str() {
            "departure_time" => "Departure time",
            "departure_date" => "Departure date",
            "flight_number" => "Flight number",
            "origin" => "Departure airport",
            "destination" => "Arrival airport",
            _ => "Flight details",
        }
    }
}

#[derive(Template)]
#[template(path = "partials/flights/card_response.html")]
pub(super) struct CardResponse {
    pub flight: data::Flight,
    pub error: String,
    pub notice: String,
}
