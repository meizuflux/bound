-- Keep a durable history from this point forward. Do not invent past events.
CREATE TABLE activity_events (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
    email TEXT NOT NULL,
    event_type TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX activity_events_user_id_id ON activity_events (user_id, id DESC);

-- Database triggers also capture role changes made through the justfile.
-- These events describe changes to an account, not who performed them.
CREATE FUNCTION record_user_activity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO activity_events (email, event_type)
        VALUES (OLD.email, 'account_deleted');
        RETURN OLD;
    END IF;

    IF TG_OP = 'INSERT' THEN
        INSERT INTO activity_events (user_id, email, event_type)
        VALUES (NEW.id, NEW.email, 'account_created');
        RETURN NEW;
    END IF;

    IF OLD.email_verified_at IS NULL AND NEW.email_verified_at IS NOT NULL THEN
        INSERT INTO activity_events (user_id, email, event_type)
        VALUES (NEW.id, NEW.email, 'email_verified');
    END IF;
    IF OLD.is_admin IS DISTINCT FROM NEW.is_admin THEN
        INSERT INTO activity_events (user_id, email, event_type)
        VALUES (NEW.id, NEW.email,
            CASE WHEN NEW.is_admin THEN 'admin_granted' ELSE 'admin_revoked' END);
    END IF;
    IF OLD.email IS DISTINCT FROM NEW.email THEN
        INSERT INTO activity_events (user_id, email, event_type)
        VALUES (NEW.id, NEW.email, 'email_changed');
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER users_activity AFTER INSERT OR UPDATE OR DELETE ON users
FOR EACH ROW EXECUTE FUNCTION record_user_activity();

CREATE FUNCTION record_session_activity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    account_id BIGINT;
BEGIN
    account_id := CASE WHEN TG_OP = 'INSERT' THEN NEW.user_id ELSE OLD.user_id END;
    -- A cascade from deleting an account is covered by account_deleted.
    INSERT INTO activity_events (user_id, email, event_type)
    SELECT id, email, CASE WHEN TG_OP = 'INSERT' THEN 'session_started' ELSE 'session_ended' END
    FROM users WHERE id = account_id;
    RETURN NULL;
END;
$$;

CREATE TRIGGER sessions_activity AFTER INSERT OR DELETE ON sessions
FOR EACH ROW EXECUTE FUNCTION record_session_activity();
