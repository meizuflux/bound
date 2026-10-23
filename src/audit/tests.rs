use super::*;
use sqlx::PgPool;

#[sqlx::test]
async fn actor_settings_are_transaction_local_and_snapshots_survive_deletion(pool: PgPool) {
    let actor: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ('actor@duke.edu', 'unused') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let subject: i64 = sqlx::query_scalar("INSERT INTO users (email, password_hash) VALUES ('subject@duke.edu', 'unused') RETURNING id")
        .fetch_one(&pool).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    set_actor(&mut transaction, actor).await.unwrap();
    sqlx::query("UPDATE users SET is_admin = true WHERE id = $1")
        .bind(subject)
        .execute(&mut *transaction)
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let attribution: (Option<i64>, Option<String>) = sqlx::query_as("SELECT actor_id, actor_email FROM activity_events WHERE user_id = $1 AND event_type = 'admin_granted'")
        .bind(subject).fetch_one(&pool).await.unwrap();
    assert_eq!(attribution, (Some(actor), Some("actor@duke.edu".into())));
    sqlx::query("UPDATE users SET is_admin = false WHERE id = $1")
        .bind(subject)
        .execute(&pool)
        .await
        .unwrap();
    let attribution: Option<i64> = sqlx::query_scalar(
        "SELECT actor_id FROM activity_events WHERE user_id = $1 AND event_type = 'admin_revoked'",
    )
    .bind(subject)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(attribution, None);
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(actor)
        .execute(&pool)
        .await
        .unwrap();
    let snapshot: (Option<i64>, Option<String>) = sqlx::query_as("SELECT actor_id, actor_email FROM activity_events WHERE user_id = $1 AND event_type = 'admin_granted'")
        .bind(subject).fetch_one(&pool).await.unwrap();
    assert_eq!(snapshot, (None, Some("actor@duke.edu".into())));
}

#[sqlx::test]
async fn explicit_flight_events_rollback_with_their_mutation(pool: PgPool) {
    let actor: i64 = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ('actor@duke.edu', 'unused') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let flight: i64 = sqlx::query_scalar("INSERT INTO flights (flight_number, departure_date, departure_time, origin, destination, timezone, departure_utc)
        VALUES ('AA123', '2026-11-24', '15:00', 'RDU', 'BOS', 'America/New_York', '2026-11-24T20:00:00Z') RETURNING id")
        .fetch_one(&pool).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    record(
        &mut transaction,
        actor,
        Some(actor),
        Some(flight),
        "flight_created",
        "Community flight created",
    )
    .await
    .unwrap();
    transaction.rollback().await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM activity_events WHERE flight_id = $1")
            .bind(flight)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let mut transaction = pool.begin().await.unwrap();
    record(
        &mut transaction,
        actor,
        Some(actor),
        Some(flight),
        "flight_created",
        "Community flight created",
    )
    .await
    .unwrap();
    transaction.commit().await.unwrap();
    let label: String =
        sqlx::query_scalar("SELECT flight_label FROM activity_events WHERE flight_id = $1")
            .bind(flight)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(label, "AA123 · 2026-11-24 · RDU → BOS");
}

#[sqlx::test]
async fn missing_actor_cannot_silently_drop_a_promised_event(pool: PgPool) {
    let mut transaction = pool.begin().await.unwrap();
    let result = record(&mut transaction, i64::MAX, None, None, "test_event", "").await;
    assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    transaction.rollback().await.unwrap();
}
