use std::fmt;

use chrono::{LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use chrono_tz::Tz;
use sqlx::{PgConnection, PgPool};

use super::catalog;

#[derive(Clone, Default)]
pub(crate) struct FlightInput {
    pub flight_number: String,
    pub departure_date: String,
    pub departure_time: String,
    pub origin: String,
    pub destination: String,
}

pub(crate) struct ValidatedFlight {
    pub flight_number: String,
    pub departure_date: String,
    pub departure_time: String,
    pub origin: String,
    pub destination: String,
    pub timezone: String,
    pub departure_utc: String,
}

#[derive(Debug)]
pub(crate) enum DomainError {
    Invalid(&'static str),
    NotFound,
    Forbidden,
    Conflict(&'static str),
    Database(sqlx::Error),
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid(message) | Self::Conflict(message) => message,
            Self::NotFound => "This flight or correction could not be found.",
            Self::Forbidden => "Your account cannot perform this action.",
            Self::Database(_) => "We couldn't save that change. Please try again.",
        })
    }
}

impl From<sqlx::Error> for DomainError {
    fn from(error: sqlx::Error) -> Self {
        if error.as_database_error().is_some_and(|error| error.is_unique_violation()) {
            Self::Conflict("A matching flight or pending correction already exists. Refresh and review it.")
        } else {
            Self::Database(error)
        }
    }
}

#[derive(Clone, sqlx::FromRow)]
pub(crate) struct Flight {
    pub id: i64,
    pub flight_number: String,
    pub departure_date: String,
    pub departure_time: String,
    pub origin: String,
    pub destination: String,
    pub timezone: String,
    pub revision: i64,
    pub attendees: i64,
    pub joined: bool,
    pub confirmed: bool,
    pub confirmations: i64,
    pub pending_corrections: i64,
    pub deleted: bool,
    pub merged_into: Option<i64>,
}

#[derive(Clone, sqlx::FromRow)]
pub(crate) struct Correction {
    pub id: i64,
    pub flight_id: i64,
    pub field: String,
    pub current_value: String,
    pub proposed_value: String,
    pub reason: String,
    pub status: String,
    pub base_revision: i64,
    pub created_at: String,
}

pub(crate) struct CreateOutcome {
    pub flight_id: i64,
    pub created: bool,
}

pub(crate) fn normalize_number(number: &str) -> Result<String, DomainError> {
    if number.len() > 24 || !number.is_ascii() {
        return Err(DomainError::Invalid("Enter a flight number such as AA123."));
    }
    let compact: String = number.chars().filter(|c| !c.is_ascii_whitespace()).map(|c| c.to_ascii_uppercase()).collect();
    if compact.len() < 3 || compact.len() > 6 || catalog::airline(&compact[..2]).is_none() {
        return Err(DomainError::Invalid("Select a supported airline and enter its flight number."));
    }
    let digits = &compact[2..];
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(DomainError::Invalid("Flight numbers must contain one to four digits after the airline code."));
    }
    let value: u16 = digits.parse().map_err(|_| DomainError::Invalid("Enter a valid flight number."))?;
    if value == 0 {
        return Err(DomainError::Invalid("Flight number must be greater than zero."));
    }
    Ok(format!("{}{value}", &compact[..2]))
}

pub(crate) fn validate_input(input: &FlightInput) -> Result<(), DomainError> {
    validate(input).map(|_| ())
}

pub(crate) fn validate(input: &FlightInput) -> Result<ValidatedFlight, DomainError> {
    let flight_number = normalize_number(&input.flight_number)?;
    let date = NaiveDate::parse_from_str(&input.departure_date, "%Y-%m-%d")
        .map_err(|_| DomainError::Invalid("Enter a valid departure date."))?;
    if date.format("%Y-%m-%d").to_string() != input.departure_date || !("2020-01-01"..="2100-12-31").contains(&input.departure_date.as_str()) {
        return Err(DomainError::Invalid("Choose a departure date between 2020 and 2100."));
    }
    let time = NaiveTime::parse_from_str(&input.departure_time, "%H:%M")
        .map_err(|_| DomainError::Invalid("Enter the scheduled departure time in HH:MM format."))?;
    if time.format("%H:%M").to_string() != input.departure_time {
        return Err(DomainError::Invalid("Enter the scheduled departure time in HH:MM format."));
    }
    let origin = catalog::airport(&input.origin).ok_or(DomainError::Invalid("Select a supported origin airport."))?;
    let destination = catalog::airport(&input.destination).ok_or(DomainError::Invalid("Select a supported destination airport."))?;
    if origin.code == destination.code {
        return Err(DomainError::Invalid("Departure and destination airports must be different."));
    }
    let timezone: Tz = origin.timezone.parse().map_err(|_| DomainError::Invalid("The airport timezone is unavailable."))?;
    let departure = match timezone.from_local_datetime(&NaiveDateTime::new(date, time)) {
        LocalResult::Single(departure) => departure,
        LocalResult::None => return Err(DomainError::Invalid("That local time does not exist because the clocks change. Check the airline's schedule.")),
        LocalResult::Ambiguous(_, _) => return Err(DomainError::Invalid("That local time occurs twice because the clocks change. Ask an admin to verify the schedule before adding this flight.")),
    };
    Ok(ValidatedFlight {
        flight_number,
        departure_date: input.departure_date.clone(),
        departure_time: input.departure_time.clone(),
        origin: origin.code.to_owned(),
        destination: destination.code.to_owned(),
        timezone: origin.timezone.to_owned(),
        departure_utc: departure.to_utc().to_rfc3339(),
    })
}

const SELECT_FLIGHTS: &str = "SELECT f.id, f.flight_number, f.departure_date::text AS departure_date,
    to_char(f.departure_time, 'HH24:MI') AS departure_time, f.origin, f.destination, f.timezone, f.revision,
    (SELECT count(*) FROM user_flights uf JOIN users u ON u.id=uf.user_id
     WHERE uf.flight_id=f.id AND lower(split_part(u.email, '@', 2))=lower(split_part(viewer.email, '@', 2))
       AND u.email_verified_at IS NOT NULL AND u.suspended_at IS NULL) AS attendees,
    EXISTS(SELECT 1 FROM user_flights uf WHERE uf.flight_id=f.id AND uf.user_id=$1) AS joined,
    EXISTS(SELECT 1 FROM flight_confirmations fc WHERE fc.flight_id=f.id AND fc.user_id=$1 AND fc.revision=f.revision) AS confirmed,
    (SELECT count(*) FROM flight_confirmations fc JOIN users u ON u.id=fc.user_id
     WHERE fc.flight_id=f.id AND fc.revision=f.revision
       AND lower(split_part(u.email, '@', 2))=lower(split_part(viewer.email, '@', 2))
       AND u.email_verified_at IS NOT NULL AND u.suspended_at IS NULL) AS confirmations,
    (SELECT count(*) FROM flight_corrections c WHERE c.flight_id=f.id AND c.status='pending') AS pending_corrections,
    f.deleted_at IS NOT NULL AS deleted, f.merged_into
    FROM flights f JOIN users viewer ON viewer.id=$1
    WHERE ($2::bigint IS NULL OR f.id=$2)
      AND ($3='detail' OR $3='admin' OR (f.deleted_at IS NULL AND f.merged_into IS NULL))
      AND ($3<>'mine' OR EXISTS(SELECT 1 FROM user_flights uf WHERE uf.flight_id=f.id AND uf.user_id=$1))
      AND ($3<>'search' OR ($4='' OR f.flight_number=$4 OR EXISTS(SELECT 1 FROM flight_aliases a WHERE a.flight_id=f.id AND a.flight_number=$4)))
      AND ($3<>'admin' OR strpos(lower(f.flight_number || ' ' || f.origin || ' ' || f.destination), lower($4))>0)
      AND ($5='' OR f.departure_date=$5::text::date)
    ORDER BY f.departure_date DESC, f.departure_time, f.id DESC LIMIT $6 OFFSET $7";

async fn list(pool: &PgPool, viewer: i64, id: Option<i64>, mode: &str, q: &str, date: &str, page: i64) -> Result<Vec<Flight>, DomainError> {
    Ok(sqlx::query_as(SELECT_FLIGHTS).bind(viewer).bind(id).bind(mode).bind(q).bind(date)
        .bind(if id.is_some() { 1_i64 } else { 26 }).bind(page.clamp(1, 10_000).saturating_sub(1) * 25).fetch_all(pool).await?)
}

pub(crate) async fn search(pool: &PgPool, user_id: i64, number: &str, date: &str, page: i64) -> Result<Vec<Flight>, DomainError> {
    let number = if number.trim().is_empty() { String::new() } else { normalize_number(number)? };
    if !date.is_empty() && (NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() || date.len()!=10) {
        return Err(DomainError::Invalid("Enter a valid departure date."));
    }
    list(pool, user_id, None, "search", &number, date, page).await
}

pub(crate) async fn my_flights(pool: &PgPool, user_id: i64, page: i64) -> Result<Vec<Flight>, DomainError> {
    list(pool, user_id, None, "mine", "", "", page).await
}

pub(crate) async fn flight(pool: &PgPool, user_id: i64, id: i64) -> Result<Option<Flight>, DomainError> {
    Ok(list(pool, user_id, Some(id), "detail", "", "", 1).await?.pop())
}

pub(crate) async fn admin_flights(pool: &PgPool, actor: i64, q: &str, page: i64) -> Result<Vec<Flight>, DomainError> {
    let mut tx=pool.begin().await?;
    authorize(&mut tx, actor, true).await?;
    tx.commit().await?;
    list(pool, actor, None, "admin", &q.chars().take(100).collect::<String>(), "", page).await
}

async fn authorize(conn: &mut PgConnection, actor: i64, admin: bool) -> Result<(), DomainError> {
    let user: Option<(String, bool, bool)> = sqlx::query_as("SELECT email, is_admin, email_verified_at IS NOT NULL AND suspended_at IS NULL FROM users WHERE id=$1 FOR SHARE")
        .bind(actor).fetch_optional(&mut *conn).await?;
    if !user.is_some_and(|(email,is_admin,verified)| verified && (!admin || is_admin) && crate::schools::from_email(&email).is_some()) {
        return Err(DomainError::Forbidden);
    }
    crate::audit::set_actor(conn, actor).await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct LockedFlight {
    flight_number: String,
    departure_date: String,
    departure_time: String,
    origin: String,
    destination: String,
    instance: i32,
    revision: i64,
    deleted: bool,
    merged_into: Option<i64>,
}

impl LockedFlight {
    fn input(&self) -> FlightInput {
        FlightInput { flight_number:self.flight_number.clone(),departure_date:self.departure_date.clone(),departure_time:self.departure_time.clone(),origin:self.origin.clone(),destination:self.destination.clone() }
    }
    fn available(&self) -> Result<(),DomainError> {
        if self.deleted || self.merged_into.is_some() { Err(DomainError::Conflict("This flight has been removed or merged. Refresh to find its current details.")) } else { Ok(()) }
    }
    fn at_revision(&self, revision:i64) -> Result<(),DomainError> {
        if self.revision!=revision { Err(DomainError::Conflict("This flight changed while you were reviewing it. Refresh and check the latest details.")) } else { Ok(()) }
    }
}

async fn locked(conn: &mut PgConnection, id:i64) -> Result<LockedFlight,DomainError> {
    sqlx::query_as("SELECT flight_number, departure_date::text AS departure_date, to_char(departure_time,'HH24:MI') AS departure_time,origin,destination,instance,revision,deleted_at IS NOT NULL AS deleted,merged_into FROM flights WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_optional(conn).await?.ok_or(DomainError::NotFound)
}

pub(crate) async fn create(pool:&PgPool,actor:i64,input:&FlightInput)->Result<CreateOutcome,DomainError> {
    create_inner(pool,actor,input,false,false).await
}

pub(crate) async fn admin_create(pool:&PgPool,actor:i64,input:&FlightInput,distinct_instance:bool)->Result<CreateOutcome,DomainError> {
    create_inner(pool,actor,input,true,distinct_instance).await
}

async fn create_inner(pool:&PgPool,actor:i64,input:&FlightInput,admin:bool,distinct:bool)->Result<CreateOutcome,DomainError> {
    let input=validate(input)?;
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,admin).await?;
    identity_lock(&mut tx).await?;
    if !distinct {
        let existing:Option<i64>=sqlx::query_scalar("SELECT f.id FROM flights f WHERE (f.flight_number=$1 OR EXISTS(SELECT 1 FROM flight_aliases a WHERE a.flight_id=f.id AND a.flight_number=$1)) AND f.departure_date=$2::text::date AND f.origin=$3 AND f.destination=$4 ORDER BY f.instance,f.id LIMIT 1")
            .bind(&input.flight_number).bind(&input.departure_date).bind(&input.origin).bind(&input.destination).fetch_optional(&mut *tx).await?;
        if let Some(id)=existing {
            let canonical:Option<(i64,bool)>=sqlx::query_as("SELECT f.id,f.deleted_at IS NOT NULL FROM flights original JOIN flights f ON f.id=coalesce(original.merged_into,original.id) WHERE original.id=$1").bind(id).fetch_optional(&mut *tx).await?;
            if canonical.is_some_and(|(_,deleted)|deleted) { return Err(DomainError::Conflict("A matching flight was removed. Ask an admin to review it before adding another.")); }
            tx.commit().await?;
            return Ok(CreateOutcome { flight_id:canonical.map_or(id,|(id,_)|id),created:false });
        }
    }
    let instance:i32=if distinct {
        sqlx::query_scalar("SELECT coalesce(max(instance)+1,0)::integer FROM flights WHERE flight_number=$1 AND departure_date=$2::text::date AND origin=$3 AND destination=$4")
            .bind(&input.flight_number).bind(&input.departure_date).bind(&input.origin).bind(&input.destination).fetch_one(&mut *tx).await?
    } else { 0 };
    let id:i64=sqlx::query_scalar("INSERT INTO flights(flight_number,departure_date,departure_time,origin,destination,timezone,departure_utc,instance,created_by) VALUES($1,$2::text::date,$3::text::time,$4,$5,$6,$7::text::timestamptz,$8,$9) RETURNING id")
        .bind(&input.flight_number).bind(&input.departure_date).bind(&input.departure_time).bind(&input.origin).bind(&input.destination).bind(&input.timezone).bind(&input.departure_utc).bind(instance).bind(actor).fetch_one(&mut *tx).await?;
    crate::audit::record(&mut tx,actor,None,Some(id),"flight_created",&format!("{} {} {} → {} at {} {}",input.flight_number,input.departure_date,input.origin,input.destination,input.departure_time,input.timezone)).await?;
    if !admin {
        sqlx::query("INSERT INTO user_flights(user_id,flight_id) VALUES($1,$2)").bind(actor).bind(id).execute(&mut *tx).await?;
        crate::audit::record(&mut tx,actor,Some(actor),Some(id),"flight_joined","").await?;
    }
    tx.commit().await?;
    Ok(CreateOutcome { flight_id:id,created:true })
}

pub(crate) async fn join(pool:&PgPool,actor:i64,id:i64)->Result<(),DomainError> {
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,false).await?;
    locked(&mut tx,id).await?.available()?;
    let result=sqlx::query("INSERT INTO user_flights(user_id,flight_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(actor).bind(id).execute(&mut *tx).await?;
    if result.rows_affected()>0 { crate::audit::record(&mut tx,actor,Some(actor),Some(id),"flight_joined","").await?; }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn leave(pool:&PgPool,actor:i64,id:i64)->Result<(),DomainError> {
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,false).await?;
    locked(&mut tx,id).await?;
    let result=sqlx::query("DELETE FROM user_flights WHERE user_id=$1 AND flight_id=$2").bind(actor).bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM flight_confirmations WHERE user_id=$1 AND flight_id=$2").bind(actor).bind(id).execute(&mut *tx).await?;
    if result.rows_affected()>0 { crate::audit::record(&mut tx,actor,Some(actor),Some(id),"flight_left","").await?; }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn confirm(pool:&PgPool,actor:i64,id:i64,revision:i64)->Result<(),DomainError> {
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,false).await?;
    let current=locked(&mut tx,id).await?;
    current.available()?;
    current.at_revision(revision)?;
    require_membership(&mut tx,actor,id).await?;
    let result=sqlx::query("INSERT INTO flight_confirmations(user_id,flight_id,revision) VALUES($1,$2,$3) ON CONFLICT DO NOTHING").bind(actor).bind(id).bind(revision).execute(&mut *tx).await?;
    if result.rows_affected()>0 { crate::audit::record(&mut tx,actor,Some(actor),Some(id),"flight_confirmed",&format!("Revision {revision}")).await?; }
    tx.commit().await?;
    Ok(())
}

async fn require_membership(conn:&mut PgConnection,actor:i64,id:i64)->Result<(),DomainError> {
    let member:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_flights WHERE user_id=$1 AND flight_id=$2)").bind(actor).bind(id).fetch_one(conn).await?;
    if member { Ok(()) } else { Err(DomainError::Invalid("Add this flight to your flights before confirming or correcting it.")) }
}

fn change(input:&mut FlightInput,field:&str,value:&str)->Result<String,DomainError> {
    let target=match field {
        "flight_number"=>&mut input.flight_number,
        "departure_date"=>&mut input.departure_date,
        "departure_time"=>&mut input.departure_time,
        "origin"=>&mut input.origin,
        "destination"=>&mut input.destination,
        _=>return Err(DomainError::Invalid("Choose a flight detail to correct.")),
    };
    let old=target.clone();
    *target=value.trim().to_owned();
    Ok(old)
}

pub(crate) async fn propose(pool:&PgPool,actor:i64,id:i64,revision:i64,field:&str,value:&str,reason:&str)->Result<(),DomainError> {
    if reason.chars().count()>500 || value.len()>50 { return Err(DomainError::Invalid("Keep the proposed value short and the explanation under 500 characters.")); }
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,false).await?;
    let current=locked(&mut tx,id).await?;
    current.available()?;
    current.at_revision(revision)?;
    require_membership(&mut tx,actor,id).await?;
    let mut input=current.input();
    let old=change(&mut input,field,value)?;
    let valid=validate(&input)?;
    let proposed=match field { "flight_number"=>valid.flight_number,"origin"=>valid.origin,"destination"=>valid.destination,_=>value.trim().to_owned() };
    if old==proposed { return Err(DomainError::Invalid("The suggested value already matches this flight.")); }
    sqlx::query("INSERT INTO flight_corrections(flight_id,proposed_by,field,current_value,proposed_value,reason,base_revision) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(id).bind(actor).bind(field).bind(&old).bind(&proposed).bind(reason.trim()).bind(revision).execute(&mut *tx).await?;
    crate::audit::record(&mut tx,actor,Some(actor),Some(id),"flight_correction_proposed",&format!("{field}: {old} → {proposed}; revision {revision}")).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn corrections(pool:&PgPool,id:i64)->Result<Vec<Correction>,DomainError> {
    Ok(sqlx::query_as("SELECT id,flight_id,field,current_value,proposed_value,reason,status,base_revision,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD HH24:MI') AS created_at FROM flight_corrections WHERE flight_id=$1 ORDER BY id DESC LIMIT 25")
        .bind(id).fetch_all(pool).await?)
}

pub(crate) async fn pending_corrections(pool:&PgPool,actor:i64,page:i64)->Result<Vec<Correction>,DomainError> {
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,true).await?;
    let rows=sqlx::query_as("SELECT id,flight_id,field,current_value,proposed_value,reason,status,base_revision,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD HH24:MI') AS created_at FROM flight_corrections WHERE status='pending' ORDER BY id LIMIT 26 OFFSET $1")
        .bind(page.clamp(1,10_000).saturating_sub(1)*25).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn update(conn:&mut PgConnection,actor:i64,id:i64,current:&LockedFlight,input:&FlightInput)->Result<(),DomainError> {
    let valid=validate(input)?;
    ensure_identity_available(conn,&valid,Some(id),current.instance).await?;
    // Existing aliases move with the flight when its route or date is corrected.
    let aliases:Vec<String>=sqlx::query_scalar("SELECT flight_number FROM flight_aliases WHERE flight_id=$1").bind(id).fetch_all(&mut *conn).await?;
    for alias in aliases {
        let alternate=ValidatedFlight { flight_number:alias,departure_date:valid.departure_date.clone(),departure_time:valid.departure_time.clone(),origin:valid.origin.clone(),destination:valid.destination.clone(),timezone:valid.timezone.clone(),departure_utc:valid.departure_utc.clone() };
        ensure_identity_available(conn,&alternate,Some(id),current.instance).await?;
    }
    sqlx::query("UPDATE flights SET flight_number=$2,departure_date=$3::text::date,departure_time=$4::text::time,origin=$5,destination=$6,timezone=$7,departure_utc=$8::text::timestamptz,revision=revision+1,updated_at=now() WHERE id=$1")
        .bind(id).bind(&valid.flight_number).bind(&valid.departure_date).bind(&valid.departure_time).bind(&valid.origin).bind(&valid.destination).bind(&valid.timezone).bind(&valid.departure_utc).execute(&mut *conn).await?;
    let details=format!("Revision {} → {}; number {} → {}; date {} → {}; departure {} → {}; route {}–{} → {}–{}",current.revision,current.revision+1,current.flight_number,valid.flight_number,current.departure_date,valid.departure_date,current.departure_time,valid.departure_time,current.origin,current.destination,valid.origin,valid.destination);
    crate::audit::record(conn,actor,None,Some(id),"flight_updated",&details).await?;
    Ok(())
}

pub(crate) async fn admin_update(pool:&PgPool,actor:i64,id:i64,revision:i64,input:&FlightInput)->Result<(),DomainError> {
    validate_input(input)?;
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,true).await?;
    identity_lock(&mut tx).await?;
    let current=locked(&mut tx,id).await?;
    current.available()?;
    current.at_revision(revision)?;
    update(&mut tx,actor,id,&current,input).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn admin_set_deleted(pool:&PgPool,actor:i64,id:i64,revision:i64,deleted:bool)->Result<(),DomainError> {
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,true).await?;
    let current=locked(&mut tx,id).await?;
    current.at_revision(revision)?;
    if current.merged_into.is_some() { return Err(DomainError::Conflict("Merged flights cannot be restored. Edit the destination flight instead.")); }
    if current.deleted!=deleted {
        sqlx::query("UPDATE flights SET deleted_at=CASE WHEN $2 THEN now() ELSE NULL END,revision=revision+1,updated_at=now() WHERE id=$1").bind(id).bind(deleted).execute(&mut *tx).await?;
        crate::audit::record(&mut tx,actor,None,Some(id),if deleted {"flight_deleted"} else {"flight_restored"},"").await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn review(pool:&PgPool,actor:i64,correction_id:i64,approve:bool)->Result<(),DomainError> {
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,true).await?;
    identity_lock(&mut tx).await?;
    // Lock flight first everywhere; correction row locking must not reverse the lock order.
    let flight_id:i64=sqlx::query_scalar("SELECT flight_id FROM flight_corrections WHERE id=$1").bind(correction_id).fetch_optional(&mut *tx).await?.ok_or(DomainError::NotFound)?;
    let current=locked(&mut tx,flight_id).await?;
    let correction:Correction=sqlx::query_as("SELECT id,flight_id,field,current_value,proposed_value,reason,status,base_revision,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD HH24:MI') AS created_at FROM flight_corrections WHERE id=$1 FOR UPDATE")
        .bind(correction_id).fetch_one(&mut *tx).await?;
    if correction.status!="pending" { return Err(DomainError::Conflict("This correction has already been reviewed.")); }
    if approve {
        current.available()?;
        current.at_revision(correction.base_revision)?;
        let mut input=current.input();
        change(&mut input,&correction.field,&correction.proposed_value)?;
        update(&mut tx,actor,flight_id,&current,&input).await?;
    }
    sqlx::query("UPDATE flight_corrections SET status=$2,reviewed_by=$3,reviewed_at=now() WHERE id=$1").bind(correction_id).bind(if approve {"approved"} else {"rejected"}).bind(actor).execute(&mut *tx).await?;
    crate::audit::record(&mut tx,actor,None,Some(flight_id),if approve {"flight_correction_approved"} else {"flight_correction_rejected"},&format!("Correction {correction_id}; {}: {} → {}",correction.field,correction.current_value,correction.proposed_value)).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn admin_merge(pool:&PgPool,actor:i64,source_id:i64,target_id:i64,source_revision:i64,target_revision:i64)->Result<(),DomainError> {
    if source_id==target_id { return Err(DomainError::Invalid("Choose two different flights to merge.")); }
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,true).await?;
    identity_lock(&mut tx).await?;
    let first=locked(&mut tx,source_id.min(target_id)).await?;
    let second=locked(&mut tx,source_id.max(target_id)).await?;
    let (source,target)=if source_id<target_id {(first,second)} else {(second,first)};
    source.available()?;
    target.available()?;
    source.at_revision(source_revision)?;
    target.at_revision(target_revision)?;
    if source.departure_date!=target.departure_date || source.origin!=target.origin || source.destination!=target.destination {
        return Err(DomainError::Conflict("Only flights with the same departure date and route can be merged. Correct those details first."));
    }
    sqlx::query("INSERT INTO user_flights(user_id,flight_id,created_at) SELECT user_id,$2,created_at FROM user_flights WHERE flight_id=$1 ON CONFLICT DO NOTHING").bind(source_id).bind(target_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM user_flights WHERE flight_id=$1").bind(source_id).execute(&mut *tx).await?;
    // Revision bump invalidates all confirmations; evidence cannot be silently reattributed.
    sqlx::query("UPDATE flights SET revision=revision+1,updated_at=now() WHERE id=$1").bind(target_id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO flight_aliases(flight_id,flight_number) SELECT $2,flight_number FROM flight_aliases WHERE flight_id=$1 UNION SELECT $2,flight_number FROM flights WHERE id=$1 ON CONFLICT DO NOTHING").bind(source_id).bind(target_id).execute(&mut *tx).await?;
    sqlx::query("UPDATE flight_corrections SET status='rejected',reviewed_by=$2,reviewed_at=now() WHERE flight_id=$1 AND status='pending'").bind(source_id).bind(actor).execute(&mut *tx).await?;
    sqlx::query("UPDATE flights SET merged_into=$2,deleted_at=now(),revision=revision+1,updated_at=now() WHERE id=$1").bind(source_id).bind(target_id).execute(&mut *tx).await?;
    // Keep redirect chains one hop, preserving links from earlier merges.
    sqlx::query("UPDATE flights SET merged_into=$2 WHERE merged_into=$1").bind(source_id).bind(target_id).execute(&mut *tx).await?;
    crate::audit::record(&mut tx,actor,None,Some(target_id),"flight_merged",&format!("Flight {source_id} revision {source_revision} merged into flight {target_id} revision {target_revision}; memberships preserved; pending source corrections rejected")).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn admin_add_alias(pool:&PgPool,actor:i64,id:i64,number:&str)->Result<(),DomainError> {
    let number=normalize_number(number)?;
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,true).await?;
    identity_lock(&mut tx).await?;
    let current=locked(&mut tx,id).await?;
    current.available()?;
    if number==current.flight_number { return Err(DomainError::Invalid("This is already the primary flight number.")); }
    let mut input=current.input();
    input.flight_number=number.clone();
    ensure_identity_available(&mut tx,&validate(&input)?,Some(id),current.instance).await?;
    sqlx::query("INSERT INTO flight_aliases(flight_id,flight_number) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(id).bind(&number).execute(&mut *tx).await?;
    crate::audit::record(&mut tx,actor,None,Some(id),"flight_alias_added",&number).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn admin_remove_alias(pool:&PgPool,actor:i64,id:i64,number:&str)->Result<(),DomainError> {
    let number=normalize_number(number)?;
    let mut tx=pool.begin().await?;
    authorize(&mut tx,actor,true).await?;
    identity_lock(&mut tx).await?;
    locked(&mut tx,id).await?.available()?;
    let result=sqlx::query("DELETE FROM flight_aliases WHERE flight_id=$1 AND flight_number=$2").bind(id).bind(&number).execute(&mut *tx).await?;
    if result.rows_affected()>0 { crate::audit::record(&mut tx,actor,None,Some(id),"flight_alias_removed",&number).await?; }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn aliases(pool:&PgPool,id:i64)->Result<Vec<String>,DomainError> {
    Ok(sqlx::query_scalar("SELECT a.flight_number FROM flight_aliases a JOIN flights f ON f.id=a.flight_id WHERE a.flight_id=$1 AND a.flight_number<>f.flight_number ORDER BY a.flight_number LIMIT 50").bind(id).fetch_all(pool).await?)
}

// A small launch catalog makes a single directory lock economical. It protects
// identity changes across the canonical and alias tables; joins remain parallel.
async fn identity_lock(conn:&mut PgConnection)->Result<(),DomainError> {
    sqlx::query("SELECT pg_advisory_xact_lock(72103002)").execute(conn).await?;
    Ok(())
}

async fn ensure_identity_available(conn:&mut PgConnection,input:&ValidatedFlight,except:Option<i64>,instance:i32)->Result<(),DomainError> {
    let collision:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM flights f WHERE ($1::bigint IS NULL OR f.id<>$1) AND f.merged_into IS NULL AND f.departure_date=$2::text::date AND f.origin=$3 AND f.destination=$4 AND f.instance=$5 AND (f.flight_number=$6 OR EXISTS(SELECT 1 FROM flight_aliases a WHERE a.flight_id=f.id AND a.flight_number=$6)))")
        .bind(except).bind(&input.departure_date).bind(&input.origin).bind(&input.destination).bind(instance).bind(&input.flight_number).fetch_one(conn).await?;
    if collision { Err(DomainError::Conflict("That flight number is already associated with another departure on this route and date. Review the matching flights or merge duplicates first.")) } else { Ok(()) }
}

#[cfg(test)]
#[path="data_tests.rs"]
mod tests;
