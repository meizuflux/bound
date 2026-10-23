ALTER TABLE users ADD COLUMN suspended_at TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN admin_version BIGINT NOT NULL DEFAULT 1;
CREATE FUNCTION bump_user_admin_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.admin_version := OLD.admin_version + 1;
    RETURN NEW;
END;
$$;
CREATE TRIGGER user_admin_version BEFORE UPDATE ON users
FOR EACH ROW EXECUTE FUNCTION bump_user_admin_version();
