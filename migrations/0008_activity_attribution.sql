ALTER TABLE activity_events
    ADD COLUMN actor_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN actor_email TEXT,
    ADD COLUMN flight_id BIGINT REFERENCES flights(id) ON DELETE SET NULL,
    ADD COLUMN flight_label TEXT,
    ADD COLUMN details TEXT NOT NULL DEFAULT '';

CREATE INDEX activity_events_flight_id_id ON activity_events (flight_id, id DESC);

CREATE FUNCTION record_account_activity(account_id BIGINT, account_email TEXT, kind TEXT)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    request_actor BIGINT := NULLIF(current_setting('bound.actor_id', true), '')::BIGINT;
BEGIN
    INSERT INTO activity_events (user_id, email, event_type, actor_id, actor_email)
    SELECT account_id, account_email, kind, actor.id, actor.email
    FROM (SELECT 1) AS placeholder
    LEFT JOIN users AS actor ON actor.id = request_actor;
END;
$$;

CREATE OR REPLACE FUNCTION record_user_activity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM record_account_activity(NULL, OLD.email, 'account_deleted');
        RETURN OLD;
    END IF;
    IF TG_OP = 'INSERT' THEN
        PERFORM record_account_activity(NEW.id, NEW.email, 'account_created');
        RETURN NEW;
    END IF;
    IF OLD.email_verified_at IS DISTINCT FROM NEW.email_verified_at THEN
        IF NEW.email_verified_at IS NOT NULL THEN
            PERFORM record_account_activity(NEW.id, NEW.email, 'email_verified');
        ELSE
            PERFORM record_account_activity(NEW.id, NEW.email, 'verification_revoked');
        END IF;
    END IF;
    IF OLD.is_admin IS DISTINCT FROM NEW.is_admin THEN
        PERFORM record_account_activity(NEW.id, NEW.email,
            CASE WHEN NEW.is_admin THEN 'admin_granted' ELSE 'admin_revoked' END);
    END IF;
    IF OLD.email IS DISTINCT FROM NEW.email THEN
        PERFORM record_account_activity(NEW.id, NEW.email, 'email_changed');
    END IF;
    -- Migration 0009 adds suspension; JSON field access keeps this trigger valid
    -- before that migration is applied, without rewriting old migrations.
    IF (to_jsonb(OLD)->>'suspended_at') IS DISTINCT FROM (to_jsonb(NEW)->>'suspended_at') THEN
        PERFORM record_account_activity(NEW.id, NEW.email,
            CASE WHEN to_jsonb(NEW)->>'suspended_at' IS NOT NULL
                THEN 'account_suspended' ELSE 'account_restored' END);
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION record_session_activity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE account_id BIGINT;
BEGIN
    account_id := CASE WHEN TG_OP = 'INSERT' THEN NEW.user_id ELSE OLD.user_id END;
    PERFORM record_account_activity(id, email,
        CASE WHEN TG_OP = 'INSERT' THEN 'session_started' ELSE 'session_ended' END)
    FROM users WHERE id = account_id;
    RETURN NULL;
END;
$$;
