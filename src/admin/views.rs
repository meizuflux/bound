use askama::Template;
use salvo::prelude::*;

use super::data::{Event, User};
use crate::templates::PageView;

pub(super) struct Nav {
    pub users: bool,
    pub activity: bool,
}

impl Nav {
    pub const USERS: Self = Self {
        users: true,
        activity: false,
    };
    pub const ACTIVITY: Self = Self {
        users: false,
        activity: true,
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
