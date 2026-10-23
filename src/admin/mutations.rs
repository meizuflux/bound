use sqlx::{PgPool, Postgres, Transaction};

use crate::{audit, schools};

#[derive(Debug)]
pub(super) enum WriteError {
    Database(sqlx::Error),
    Conflict(&'static str),
    Missing,
}
impl From<sqlx::Error> for WriteError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

async fn begin(pool: &PgPool, actor: i64) -> Result<Transaction<'_, Postgres>, WriteError> {
    let mut tx = pool.begin().await?;
    // Serialize privileged mutations so two concurrent requests cannot remove the last admin.
    sqlx::query("SELECT pg_advisory_xact_lock(72103001)")
        .execute(&mut *tx)
        .await?;
    let account: Option<(String, bool)> = sqlx::query_as("SELECT email, is_admin AND email_verified_at IS NOT NULL AND suspended_at IS NULL FROM users WHERE id=$1 FOR SHARE")
        .bind(actor).fetch_optional(&mut *tx).await?;
    if !account.is_some_and(|(email, allowed)| allowed && schools::from_email(&email).is_some()) {
        return Err(WriteError::Conflict(
            "Your admin access changed. Reload this page.",
        ));
    }
    audit::set_actor(&mut tx, actor).await?;
    Ok(tx)
}

pub(super) async fn create(
    pool: &PgPool,
    actor: i64,
    email: &str,
    password_hash: &str,
    is_admin: bool,
) -> Result<i64, WriteError> {
    let mut tx = begin(pool, actor).await?;
    let inserted = sqlx::query_scalar(
        "INSERT INTO users(email,password_hash,is_admin) VALUES($1,$2,$3) RETURNING id",
    )
    .bind(email)
    .bind(password_hash)
    .bind(is_admin)
    .fetch_one(&mut *tx)
    .await;
    let id = match inserted {
        Err(error)
            if error
                .as_database_error()
                .is_some_and(|e| e.is_unique_violation()) =>
        {
            return Err(WriteError::Conflict("An account already uses that email."));
        }
        other => other?,
    };
    tx.commit().await?;
    Ok(id)
}

pub(super) async fn update(
    pool: &PgPool,
    actor: i64,
    id: i64,
    version: i64,
    is_admin: bool,
    suspended: bool,
    revoke_sessions: bool,
) -> Result<(), WriteError> {
    let mut tx = begin(pool, actor).await?;
    protect_target(&mut tx, actor, id, version, !is_admin || suspended).await?;
    sqlx::query("UPDATE users SET is_admin=$2, suspended_at=CASE WHEN $3 THEN coalesce(suspended_at,now()) ELSE NULL END WHERE id=$1").bind(id).bind(is_admin).bind(suspended).execute(&mut *tx).await?;
    if suspended || revoke_sessions {
        sqlx::query("DELETE FROM sessions WHERE user_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if revoke_sessions {
            audit::record(
                &mut tx,
                actor,
                Some(id),
                None,
                "sessions_revoked",
                "Admin revoked all active sessions",
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn protect_target(
    tx: &mut Transaction<'_, Postgres>,
    actor: i64,
    id: i64,
    version: i64,
    reducing_access: bool,
) -> Result<(), WriteError> {
    let target: Option<(i64,bool,bool,bool,String)> = sqlx::query_as("SELECT admin_version,is_admin,email_verified_at IS NOT NULL,suspended_at IS NOT NULL,email FROM users WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut **tx).await?;
    let Some((actual, is_admin, verified, suspended, email)) = target else {
        return Err(WriteError::Missing);
    };
    if version != actual {
        return Err(WriteError::Conflict(
            "This account changed. Reload the page before saving.",
        ));
    }
    if reducing_access && actor == id {
        return Err(WriteError::Conflict(
            "You cannot remove your own admin access, suspend, or delete your account.",
        ));
    }
    if reducing_access
        && is_admin
        && verified
        && !suspended
        && schools::from_email(&email).is_some()
    {
        let admin_emails: Vec<String> = sqlx::query_scalar("SELECT email FROM users WHERE is_admin AND email_verified_at IS NOT NULL AND suspended_at IS NULL").fetch_all(&mut **tx).await?;
        if admin_emails
            .iter()
            .filter(|email| schools::from_email(email).is_some())
            .count()
            <= 1
        {
            return Err(WriteError::Conflict(
                "Keep at least one active, verified admin account.",
            ));
        }
    }
    Ok(())
}

pub(super) async fn delete(
    pool: &PgPool,
    actor: i64,
    id: i64,
    version: i64,
) -> Result<(), WriteError> {
    let mut tx = begin(pool, actor).await?;
    protect_target(&mut tx, actor, id, version, true).await?;
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
