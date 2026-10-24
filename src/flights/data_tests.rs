use super::*;

fn input() -> FlightInput {
    FlightInput {
        flight_number: " aa 00123 ".into(),
        departure_date: "2027-06-10".into(),
        departure_time: "15:00".into(),
        origin: "rdu".into(),
        destination: "LAX".into(),
    }
}

async fn user(pool: &PgPool, email: &str, admin: bool) -> i64 {
    sqlx::query_scalar("INSERT INTO users(email,password_hash,email_verified_at,is_admin) VALUES($1,'test-only-unused-hash',now(),$2) RETURNING id")
        .bind(email).bind(admin).fetch_one(pool).await.unwrap()
}

#[test]
fn normalization_validation_and_airport_timezone_are_consistent() {
    let mut draft = input();
    let valid = validate(&draft).unwrap();
    assert_eq!(valid.flight_number, "AA123");
    assert_eq!(valid.origin, "RDU");
    assert_eq!(valid.timezone, "America/New_York");
    assert_eq!(valid.departure_utc, "2027-06-10T19:00:00+00:00");
    for number in [
        "AA0",
        "AA-123",
        "AA12345",
        "EV123",
        "éé123",
        "AA123' OR 1=1",
    ] {
        assert!(normalize_number(number).is_err(), "{number}");
    }
    draft.origin = "LAX".into();
    assert!(validate(&draft).is_err());
    draft.origin = "ZZZ".into();
    assert!(validate(&draft).is_err());
    draft = input();
    draft.departure_date = "2027-02-30".into();
    assert!(validate(&draft).is_err());
    draft.departure_date = "2027-03-14".into();
    draft.departure_time = "02:30".into();
    assert!(validate(&draft).is_err());
    draft.departure_date = "2027-11-07".into();
    draft.departure_time = "01:30".into();
    assert!(validate(&draft).is_err());
    for airport in catalog::AIRPORTS {
        assert!(airport.timezone.parse::<Tz>().is_ok());
    }
}

#[sqlx::test]
async fn concurrent_creation_deduplicates_different_times_without_silent_join(pool: PgPool) {
    let first = user(&pool, "first@duke.edu", false).await;
    let second = user(&pool, "second@duke.edu", false).await;
    let a = input();
    let mut b = input();
    b.departure_time = "17:00".into();
    let (one, two) = tokio::join!(create(&pool, first, &a), create(&pool, second, &b));
    let one = one.unwrap();
    let two = two.unwrap();
    assert_eq!(one.flight_id, two.flight_id);
    assert_ne!(one.created, two.created);
    let memberships: i64 = sqlx::query_scalar("SELECT count(*) FROM user_flights")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(memberships, 1);
    join(&pool, first, one.flight_id).await.unwrap();
    join(&pool, second, one.flight_id).await.unwrap();
    join(&pool, second, one.flight_id).await.unwrap();
    let matches = search(&pool, second, "AA0123", "2027-06-10", 1)
        .await
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].attendees, 2);
    assert_eq!(matches[0].confirmed, two.created);
    assert_eq!(matches[0].confirmations, 0);
}

#[sqlx::test]
async fn corrections_require_membership_and_stale_review_preserves_revision(pool: PgPool) {
    let admin = user(&pool, "admin@duke.edu", true).await;
    let student = user(&pool, "student@duke.edu", false).await;
    let outsider = user(&pool, "outsider@duke.edu", false).await;
    let id = create(&pool, student, &input()).await.unwrap().flight_id;
    assert!(
        propose(&pool, outsider, id, 1, "departure_time", "16:00", "")
            .await
            .is_err()
    );
    propose(
        &pool,
        student,
        id,
        1,
        "departure_time",
        "16:00",
        "Ticket schedule",
    )
    .await
    .unwrap();
    propose(&pool, student, id, 1, "departure_time", "17:00", "")
        .await
        .unwrap();
    let pending = pending_corrections(&pool, admin, 1).await.unwrap();
    confirm(&pool, student, id, 1).await.unwrap();
    assert!(flight(&pool, student, id).await.unwrap().unwrap().confirmed);
    review(&pool, admin, pending[0].id, true).await.unwrap();
    let current = flight(&pool, student, id).await.unwrap().unwrap();
    assert_eq!(current.departure_time, "16:00");
    assert_eq!(current.revision, 2);
    assert_eq!(current.confirmations, 0);
    assert!(!current.confirmed);
    assert!(matches!(
        review(&pool, admin, pending[1].id, true).await,
        Err(DomainError::Conflict(_))
    ));
    assert!(matches!(
        confirm(&pool, student, id, 1).await,
        Err(DomainError::Conflict(_))
    ));
    assert_eq!(
        flight(&pool, student, id)
            .await
            .unwrap()
            .unwrap()
            .departure_time,
        "16:00"
    );
    review(&pool, admin, pending[1].id, false).await.unwrap();
    assert!(
        pending_corrections(&pool, admin, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test]
async fn merges_preserve_memberships_aliases_and_redirects_without_reusing_confirmations(
    pool: PgPool,
) {
    let admin = user(&pool, "admin@duke.edu", true).await;
    let first = user(&pool, "first@duke.edu", false).await;
    let second = user(&pool, "second@duke.edu", false).await;
    let source = create(&pool, first, &input()).await.unwrap().flight_id;
    let mut other = input();
    other.flight_number = "UA456".into();
    let target = create(&pool, second, &other).await.unwrap().flight_id;
    join(&pool, first, target).await.unwrap();
    confirm(&pool, first, source, 1).await.unwrap();
    confirm(&pool, second, target, 1).await.unwrap();
    propose(&pool, first, source, 1, "departure_time", "16:00", "")
        .await
        .unwrap();
    admin_merge(&pool, admin, source, target, 1, 1)
        .await
        .unwrap();
    let merged = flight(&pool, first, source).await.unwrap().unwrap();
    assert_eq!(merged.merged_into, Some(target));
    let current = flight(&pool, first, target).await.unwrap().unwrap();
    assert_eq!(current.attendees, 2);
    assert!(current.joined);
    assert_eq!(current.confirmations, 0);
    assert_eq!(aliases(&pool, target).await.unwrap(), vec!["AA123"]);
    let results = search(&pool, first, "AA123", "2027-06-10", 1)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, target);
    assert!(
        pending_corrections(&pool, admin, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(join(&pool, second, source).await.is_err());
    let original = create(&pool, second, &input()).await.unwrap();
    assert!(!original.created);
    assert_eq!(original.flight_id, target);
}

#[sqlx::test]
async fn authorization_soft_delete_and_distinct_departures_are_enforced(pool: PgPool) {
    let admin = user(&pool, "admin@duke.edu", true).await;
    let student = user(&pool, "student@duke.edu", false).await;
    let unsupported = user(&pool, "outside@example.com", false).await;
    let id = create(&pool, student, &input()).await.unwrap().flight_id;
    assert!(matches!(
        create(&pool, unsupported, &input()).await,
        Err(DomainError::Forbidden)
    ));
    assert!(matches!(
        admin_set_deleted(&pool, student, id, 1, true).await,
        Err(DomainError::Forbidden)
    ));
    assert!(matches!(
        admin_merge(&pool, admin, id, id, 1, 1).await,
        Err(DomainError::Invalid(_))
    ));
    let distinct = admin_create(&pool, admin, &input(), true).await.unwrap();
    assert!(distinct.created);
    assert_ne!(id, distinct.flight_id);
    admin_set_deleted(&pool, admin, id, 1, true).await.unwrap();
    assert!(join(&pool, student, id).await.is_err());
    assert_eq!(
        search(&pool, student, "AA123", "2027-06-10", 1)
            .await
            .unwrap()
            .len(),
        1
    );
    admin_set_deleted(&pool, admin, id, 2, false).await.unwrap();
    assert_eq!(
        search(&pool, student, "AA123", "2027-06-10", 1)
            .await
            .unwrap()
            .len(),
        2
    );
    sqlx::query("UPDATE users SET suspended_at=now() WHERE id=$1")
        .bind(student)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        join(&pool, student, id).await,
        Err(DomainError::Forbidden)
    ));
    assert!(matches!(
        confirm(&pool, student, id, 3).await,
        Err(DomainError::Forbidden)
    ));
}

#[sqlx::test]
async fn aliases_are_searchable_removable_and_admin_only(pool: PgPool) {
    let admin = user(&pool, "admin@duke.edu", true).await;
    let student = user(&pool, "student@duke.edu", false).await;
    let id = create(&pool, student, &input()).await.unwrap().flight_id;
    assert!(matches!(
        admin_add_alias(&pool, student, id, "UA99").await,
        Err(DomainError::Forbidden)
    ));
    admin_add_alias(&pool, admin, id, "UA99").await.unwrap();
    assert_eq!(
        search(&pool, student, "UA99", "2027-06-10", 1)
            .await
            .unwrap()[0]
            .id,
        id
    );
    let mut alternate = input();
    alternate.flight_number = "UA99".into();
    let outcome = create(&pool, student, &alternate).await.unwrap();
    assert_eq!(outcome.flight_id, id);
    assert!(!outcome.created);
    admin_remove_alias(&pool, admin, id, "UA99").await.unwrap();
    assert!(
        search(&pool, student, "UA99", "2027-06-10", 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test]
async fn merged_history_does_not_reserve_a_former_departure_identity(pool: PgPool) {
    let admin = user(&pool, "admin@duke.edu", true).await;
    let student = user(&pool, "student@duke.edu", false).await;
    let source = create(&pool, student, &input()).await.unwrap().flight_id;
    let mut target_input = input();
    target_input.flight_number = "UA456".into();
    let target = admin_create(&pool, admin, &target_input, false)
        .await
        .unwrap()
        .flight_id;
    admin_merge(&pool, admin, source, target, 1, 1)
        .await
        .unwrap();
    target_input.departure_date = "2027-06-11".into();
    admin_update(&pool, admin, target, 2, &target_input)
        .await
        .unwrap();
    let recreated = create(&pool, student, &input()).await.unwrap();
    assert!(recreated.created);
    assert_ne!(recreated.flight_id, source);
    assert_ne!(recreated.flight_id, target);
    assert_eq!(
        flight(&pool, student, source)
            .await
            .unwrap()
            .unwrap()
            .merged_into,
        Some(target)
    );
}

#[sqlx::test]
async fn conflicting_aliases_and_unrelated_merges_are_rejected_atomically(pool: PgPool) {
    let admin = user(&pool, "admin@duke.edu", true).await;
    let student = user(&pool, "student@duke.edu", false).await;
    let first = create(&pool, student, &input()).await.unwrap().flight_id;
    let mut second_input = input();
    second_input.flight_number = "UA456".into();
    let second = admin_create(&pool, admin, &second_input, false)
        .await
        .unwrap()
        .flight_id;
    assert!(matches!(
        admin_add_alias(&pool, admin, first, "UA456").await,
        Err(DomainError::Conflict(_))
    ));
    admin_add_alias(&pool, admin, first, "DL789").await.unwrap();
    second_input.flight_number = "DL789".into();
    assert!(matches!(
        admin_update(&pool, admin, second, 1, &second_input).await,
        Err(DomainError::Conflict(_))
    ));
    second_input.flight_number = "UA456".into();
    second_input.departure_date = "2027-06-11".into();
    admin_update(&pool, admin, second, 1, &second_input)
        .await
        .unwrap();
    assert!(matches!(
        admin_merge(&pool, admin, first, second, 1, 2).await,
        Err(DomainError::Conflict(_))
    ));
    assert!(
        flight(&pool, student, first)
            .await
            .unwrap()
            .unwrap()
            .merged_into
            .is_none()
    );
    assert!(flight(&pool, student, first).await.unwrap().unwrap().joined);
}

#[sqlx::test]
async fn archived_memberships_remain_visible_but_cannot_be_recreated(pool: PgPool) {
    let admin = user(&pool, "admin@duke.edu", true).await;
    let student = user(&pool, "student@duke.edu", false).await;
    let id = create(&pool, student, &input()).await.unwrap().flight_id;
    admin_set_deleted(&pool, admin, id, 1, true).await.unwrap();
    let mine = my_flights(&pool, student, 1).await.unwrap();
    assert_eq!(mine.len(), 1);
    assert!(mine[0].deleted);
    assert!(matches!(
        create(&pool, student, &input()).await,
        Err(DomainError::Conflict(_))
    ));
    leave(&pool, student, id).await.unwrap();
    assert!(my_flights(&pool, student, 1).await.unwrap().is_empty());
}

#[sqlx::test]
async fn creator_initial_confirmation_is_implicit_and_only_others_add_votes(pool: PgPool) {
    let creator = user(&pool, "creator@duke.edu", false).await;
    let traveler = user(&pool, "traveler@duke.edu", false).await;
    let id = create(&pool, creator, &input()).await.unwrap().flight_id;
    confirm(&pool, creator, id, 1).await.unwrap();
    let initial = flight(&pool, creator, id).await.unwrap().unwrap();
    assert!(initial.confirmed);
    assert_eq!(initial.confirmations, 0);
    join(&pool, traveler, id).await.unwrap();
    assert!(
        !flight(&pool, traveler, id)
            .await
            .unwrap()
            .unwrap()
            .confirmed
    );
    confirm(&pool, traveler, id, 1).await.unwrap();
    confirm(&pool, traveler, id, 1).await.unwrap();
    // Old creator confirmations must not inflate counts on pre-existing flights.
    sqlx::query("INSERT INTO flight_confirmations(user_id,flight_id,revision) VALUES($1,$2,1)")
        .bind(creator)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        flight(&pool, creator, id)
            .await
            .unwrap()
            .unwrap()
            .confirmations,
        1
    );
    sqlx::query("UPDATE flights SET revision=2 WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!flight(&pool, creator, id).await.unwrap().unwrap().confirmed);
    confirm(&pool, creator, id, 2).await.unwrap();
    let revised = flight(&pool, creator, id).await.unwrap().unwrap();
    assert!(revised.confirmed);
    assert_eq!(revised.confirmations, 1);
}
