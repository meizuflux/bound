CREATE TABLE flights (
    id BIGSERIAL PRIMARY KEY,
    flight_number TEXT NOT NULL,
    departure_date DATE NOT NULL,
    departure_time TIME NOT NULL,
    origin TEXT NOT NULL CHECK (origin ~ '^[A-Z]{3}$'),
    destination TEXT NOT NULL CHECK (destination ~ '^[A-Z]{3}$' AND destination <> origin),
    timezone TEXT NOT NULL,
    departure_utc TIMESTAMPTZ NOT NULL,
    instance INTEGER NOT NULL DEFAULT 0 CHECK (instance >= 0),
    revision BIGINT NOT NULL DEFAULT 1,
    created_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ,
    merged_into BIGINT REFERENCES flights(id),
    CHECK (merged_into IS NULL OR merged_into <> id),
    UNIQUE (flight_number, departure_date, origin, destination, instance)
);
CREATE INDEX flights_departure ON flights (departure_date, id) WHERE deleted_at IS NULL AND merged_into IS NULL;

CREATE TABLE user_flights (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    flight_id BIGINT NOT NULL REFERENCES flights(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, flight_id)
);
CREATE INDEX user_flights_flight ON user_flights (flight_id, user_id);

CREATE TABLE flight_confirmations (
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    flight_id BIGINT NOT NULL REFERENCES flights(id),
    revision BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, flight_id, revision)
);
CREATE INDEX flight_confirmations_revision ON flight_confirmations (flight_id, revision);

CREATE TABLE flight_corrections (
    id BIGSERIAL PRIMARY KEY,
    flight_id BIGINT NOT NULL REFERENCES flights(id),
    proposed_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    field TEXT NOT NULL CHECK (field IN ('flight_number', 'departure_date', 'departure_time', 'origin', 'destination')),
    current_value TEXT NOT NULL,
    proposed_value TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    base_revision BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'rejected')),
    reviewed_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reviewed_at TIMESTAMPTZ
);
CREATE INDEX flight_corrections_pending ON flight_corrections (id) WHERE status = 'pending';
CREATE INDEX flight_corrections_flight ON flight_corrections (flight_id, id DESC);
CREATE UNIQUE INDEX flight_corrections_duplicate ON flight_corrections (flight_id, proposed_by, field, proposed_value, base_revision) WHERE status = 'pending';

CREATE TABLE flight_aliases (
    flight_id BIGINT NOT NULL REFERENCES flights(id),
    flight_number TEXT NOT NULL,
    PRIMARY KEY (flight_id, flight_number)
);
CREATE INDEX flight_aliases_number ON flight_aliases (flight_number, flight_id);
