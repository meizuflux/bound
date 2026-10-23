use sqlx::PgConnection;

/// Bind an authenticated actor to this transaction for database-triggered events.
/// The setting is transaction-local and cannot leak to another pooled request.
pub(crate) async fn set_actor(
    connection: &mut PgConnection,
    actor_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config('bound.actor_id', $1, true)")
        .bind(actor_id.to_string())
        .execute(connection)
        .await?;
    Ok(())
}

/// Record a committed domain change, retaining readable snapshots after deletion.
/// Call within the same transaction as the change; never pass credentials here.
pub(crate) async fn record(
    connection: &mut PgConnection,
    actor_id: i64,
    subject_user_id: Option<i64>,
    flight_id: Option<i64>,
    event_type: &str,
    details: &str,
) -> Result<(), sqlx::Error> {
    let details: String = details.chars().take(1000).collect();
    sqlx::query(
        "INSERT INTO activity_events
            (user_id, email, actor_id, actor_email, flight_id, flight_label, event_type, details)
         SELECT subject.id, COALESCE(subject.email, actor.email), actor.id, actor.email,
            flight.id,
            CASE WHEN flight.id IS NOT NULL THEN flight.flight_number || ' · ' ||
                flight.departure_date::text || ' · ' || flight.origin || ' → ' || flight.destination END,
            $4, $5
         FROM users AS actor
         LEFT JOIN users AS subject ON subject.id = $2
         LEFT JOIN flights AS flight ON flight.id = $3
         WHERE actor.id = $1",
    )
    .bind(actor_id)
    .bind(subject_user_id)
    .bind(flight_id)
    .bind(event_type)
    .bind(details)
    .execute(connection)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
