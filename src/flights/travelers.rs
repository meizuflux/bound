use sqlx::PgPool;

#[derive(sqlx::FromRow)]
pub(super) struct Traveler {
    pub name: String,
    pub phone: String,
    pub is_you: bool,
}

/// Keep the audience check and phone projection in SQL: private contact details
/// must never enter the rendered view, including status polls and mutation replies.
pub(super) async fn list(
    pool: &PgPool,
    viewer: i64,
    flight: i64,
) -> Result<Vec<Traveler>, sqlx::Error> {
    sqlx::query_as(
        "SELECT COALESCE(NULLIF(u.display_name, ''), 'Fellow student') AS name,
            CASE WHEN u.share_phone THEN u.phone_number ELSE '' END AS phone,
            u.id=$1 AS is_you
         FROM users viewer
         JOIN user_flights mine ON mine.user_id=viewer.id AND mine.flight_id=$2
         JOIN flights f ON f.id=mine.flight_id AND f.deleted_at IS NULL AND f.merged_into IS NULL
         JOIN user_flights members ON members.flight_id=f.id
         JOIN users u ON u.id=members.user_id
         WHERE viewer.id=$1 AND viewer.email_verified_at IS NOT NULL AND viewer.suspended_at IS NULL
           AND lower(split_part(viewer.email, '@', 2))=ANY($3)
           AND lower(split_part(u.email, '@', 2))=lower(split_part(viewer.email, '@', 2))
           AND u.email_verified_at IS NOT NULL AND u.suspended_at IS NULL AND u.profile_visible
         ORDER BY (u.id=$1) DESC, lower(u.display_name), u.id LIMIT 101",
    )
    .bind(viewer)
    .bind(flight)
    .bind(
        crate::schools::SCHOOLS
            .iter()
            .flat_map(|school| school.email_suffixes.iter().copied())
            .collect::<Vec<_>>(),
    )
    .fetch_all(pool)
    .await
}
