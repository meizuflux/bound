use askama::Template;
use salvo::prelude::*;

use super::data::{Event, User};
use crate::templates::PageView;

pub(super) struct Nav {
    pub users: bool,
    pub activity: bool,
    pub flights: bool,
    pub corrections: bool,
}

impl Nav {
    pub const USERS: Self = Self {
        users: true,
        activity: false,
        flights: false,
        corrections: false,
    };
    pub const ACTIVITY: Self = Self {
        users: false,
        activity: true,
        flights: false,
        corrections: false,
    };
    pub const FLIGHTS: Self = Self {
        users: false,
        activity: false,
        flights: true,
        corrections: false,
    };
    pub const CORRECTIONS: Self = Self {
        users: false,
        activity: false,
        flights: false,
        corrections: true,
    };
}

pub(super) struct Pagination {
    pub q: String,
    pub page: i64,
    pub has_next: bool,
}

impl Pagination {
    pub const LIMIT: i64 = 25;

    pub fn from_request(req: &Request) -> Result<Self, StatusError> {
        let page = match req.query::<String>("page") {
            None => 1,
            Some(value) => value
                .parse::<i64>()
                .ok()
                .filter(|p| (1..=1_000_000).contains(p))
                .ok_or_else(StatusError::bad_request)?,
        };
        let q = req
            .query::<String>("q")
            .unwrap_or_default()
            .trim()
            .to_owned();
        if q.len() > 254 {
            return Err(StatusError::bad_request());
        }
        Ok(Self {
            q,
            page,
            has_next: false,
        })
    }

    pub fn offset(&self) -> i64 {
        (self.page - 1) * Self::LIMIT
    }

    pub fn finish<T>(&mut self, rows: &mut Vec<T>) {
        self.has_next = rows.len() > Self::LIMIT as usize;
        rows.truncate(Self::LIMIT as usize);
    }
}

#[derive(Template)]
#[template(path = "partials/admin/users.html")]
pub(super) struct UsersPage {
    pub nav: Nav,
    pub users: Vec<User>,
    pub pagination: Pagination,
}

impl PageView for UsersPage {
    fn title(&self) -> &'static str {
        "Users · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/user.html")]
pub(super) struct UserPage {
    pub nav: Nav,
    pub user: User,
    pub events: Vec<Event>,
    pub message: String,
    pub error: String,
    pub own_account: bool,
}

impl PageView for UserPage {
    fn title(&self) -> &'static str {
        "User details · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/activity.html")]
pub(super) struct ActivityPage {
    pub nav: Nav,
    pub events: Vec<Event>,
    pub pagination: Pagination,
}

impl PageView for ActivityPage {
    fn title(&self) -> &'static str {
        "Activity history · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/user_form.html")]
pub(super) struct UserForm {
    pub nav: Nav,
    pub email: String,
    pub is_admin: bool,
    pub error: String,
}
impl PageView for UserForm {
    fn title(&self) -> &'static str {
        "Create user · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/user_delete.html")]
pub(super) struct UserDelete {
    pub nav: Nav,
    pub user: User,
    pub error: String,
}
impl PageView for UserDelete {
    fn title(&self) -> &'static str {
        "Delete user · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/flights.html")]
pub(super) struct FlightsPage {
    pub nav: Nav,
    pub flights: Vec<crate::flights::data::Flight>,
    pub pagination: Pagination,
}
impl PageView for FlightsPage {
    fn title(&self) -> &'static str {
        "Flights · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/flight_form.html")]
pub(super) struct FlightForm {
    pub nav: Nav,
    pub id: i64,
    pub revision: i64,
    pub input: crate::flights::data::FlightInput,
    pub error: String,
    pub deleted: bool,
    pub attendees: i64,
    pub message: String,
    pub aliases: Vec<String>,
    pub merged_into: Option<i64>,
}
impl PageView for FlightForm {
    fn title(&self) -> &'static str {
        "Flight · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/corrections.html")]
pub(super) struct CorrectionsPage {
    pub nav: Nav,
    pub corrections: Vec<crate::flights::data::Correction>,
    pub pagination: Pagination,
    pub error: String,
}
impl PageView for CorrectionsPage {
    fn title(&self) -> &'static str {
        "Flight corrections · Admin"
    }
}

#[derive(Template)]
#[template(path = "partials/admin/merge.html")]
pub(super) struct MergePage {
    pub nav: Nav,
    pub source: crate::flights::data::Flight,
    pub target: Option<crate::flights::data::Flight>,
    pub error: String,
}
impl PageView for MergePage {
    fn title(&self) -> &'static str {
        "Merge flights · Admin"
    }
}
