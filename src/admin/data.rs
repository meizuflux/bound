use sqlx::PgPool;

use super::views::Pagination;
use crate::schools;

#[derive(sqlx::FromRow)]
pub(super) struct User {
    pub id: i64,
    pub email: String,
    pub is_admin: bool,
    pub verified: bool,
    pub suspended: bool,
    pub version: i64,
    pub created_at: String,
}

impl User {
    pub fn school(&self) -> &'static str {
        schools::from_email(&self.email).map_or("Unsupported school", |school| school.name)
    }

    pub fn status(&self) -> &'static str {
        if self.suspended {
            "Suspended"
        } else if schools::from_email(&self.email).is_none() {
            "Unsupported email"
        } else if self.verified {
            "Verified"
        } else {
            "Pending verification"
        }
    }

    pub fn role(&self) -> &'static str {
        if self.is_admin { "Admin" } else { "Student" }
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct Event {
    pub user_id: Option<i64>,
    pub email: String,
    pub event_type: String,
    pub actor_email: Option<String>,
    pub flight_id: Option<i64>,
    pub flight_label: Option<String>,
    pub details: String,
    pub created_at: String,
}

impl Event {
    pub fn label(&self) -> &str {
        match self.event_type.as_str() {
            "profile_updated" => "Profile updated",
            "account_created" => "Account created",
            "account_suspended" => "Account suspended",
            "account_restored" => "Account restored",
            "sessions_revoked" => "Sessions revoked",
            "flight_created" => "Flight created",
            "flight_updated" => "Flight updated",
            "flight_deleted" => "Flight archived",
            "flight_restored" => "Flight restored",
            "flight_joined" => "Flight added to trips",
            "flight_left" => "Flight removed from trips",
            "flight_confirmed" => "Flight details confirmed",
            "flight_merged" => "Duplicate flights merged",
            "flight_correction_proposed" => "Flight correction suggested",
            "flight_correction_approved" => "Flight correction approved",
            "flight_correction_rejected" => "Flight correction rejected",
            "flight_alias_added" => "Alternate flight number added",
            "flight_alias_removed" => "Alternate flight number removed",
            "account_deleted" => "Account deleted",
            "email_verified" => "Email verified",
            "email_changed" => "Email changed",
            "admin_granted" => "Admin access granted",
            "admin_revoked" => "Admin access revoked",
            "session_started" => "Session started",
            "session_ended" => "Session ended",
            _ => &self.event_type,
        }
    }
}

pub(super) async fn users(pool: &PgPool, page: &Pagination) -> Result<Vec<User>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, email, is_admin, email_verified_at IS NOT NULL AS verified, suspended_at IS NOT NULL AS suspended, admin_version AS version,
        to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') AS created_at
        FROM users WHERE strpos(lower(email), lower($1)) > 0 ORDER BY id DESC LIMIT $2 OFFSET $3",
    )
    .bind(&page.q)
    .bind(Pagination::LIMIT + 1)
    .bind(page.offset())
    .fetch_all(pool)
    .await
}

pub(super) async fn user(pool: &PgPool, id: i64) -> Result<Option<User>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, email, is_admin, email_verified_at IS NOT NULL AS verified, suspended_at IS NOT NULL AS suspended, admin_version AS version,
        to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI') AS created_at
        FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub(super) async fn activity(pool: &PgPool, page: &Pagination) -> Result<Vec<Event>, sqlx::Error> {
    sqlx::query_as("SELECT user_id, email, event_type, actor_email, flight_id, flight_label, details,
        to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS created_at
        FROM activity_events WHERE strpos(lower(email || ' ' || coalesce(actor_email,'') || ' ' || coalesce(flight_label,'') || ' ' || event_type || ' ' || details), lower($1)) > 0 ORDER BY id DESC LIMIT $2 OFFSET $3")
        .bind(&page.q).bind(Pagination::LIMIT + 1).bind(page.offset()).fetch_all(pool).await
}

pub(super) async fn user_activity(pool: &PgPool, id: i64) -> Result<Vec<Event>, sqlx::Error> {
    sqlx::query_as(
        "SELECT user_id, email, event_type, actor_email, flight_id, flight_label, details,
        to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS') AS created_at
        FROM activity_events WHERE user_id = $1 ORDER BY id DESC LIMIT 25",
    )
    .bind(id)
    .fetch_all(pool)
    .await
}
