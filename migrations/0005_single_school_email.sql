-- Run transactionally: preserve accounts and use only proven school addresses
-- as their login emails. Never merge users or adopt an unverified destination.
LOCK TABLE users, email_verifications IN ACCESS EXCLUSIVE MODE;

DO $$
BEGIN
    -- This migration snapshots the supported school map at the time of 0005.
    -- Do not drop the old school-id check while trusting inconsistent data.
    IF EXISTS (
        SELECT 1 FROM users
        WHERE school_email_verified_at IS NOT NULL
          AND (school IS DISTINCT FROM 'duke'
               OR school_email IS NULL
               OR school_email <> lower(btrim(school_email))
               OR school_email !~ '^[^@[:space:]]+@duke[.]edu$')
    ) THEN
        RAISE EXCEPTION 'Cannot migrate inconsistent verified school emails; review the legacy school mappings before retrying';
    END IF;

    IF EXISTS (
        SELECT 1 FROM users AS verified
        JOIN users AS existing
          ON lower(btrim(existing.email)) = verified.school_email
         AND existing.id <> verified.id
        WHERE verified.school_email_verified_at IS NOT NULL
    ) THEN
        RAISE EXCEPTION 'Cannot migrate school emails: a verified address is already another account''s login email; resolve the conflict before retrying (no accounts were merged or deleted)';
    END IF;
END $$;

UPDATE users SET email = school_email
WHERE school_email_verified_at IS NOT NULL;

-- Old links must never verify an email they were not actually sent to.
-- Matching, supported legacy links remain valid and single-use.
DELETE FROM email_verifications AS verification USING users
WHERE verification.user_id = users.id
  AND (verification.school_email IS DISTINCT FROM users.email
       OR verification.school IS DISTINCT FROM 'duke'
       OR verification.school_email !~ '^[^@[:space:]]+@duke[.]edu$'
       OR users.school_email_verified_at IS NOT NULL);

ALTER TABLE users DROP CONSTRAINT verified_school_present;
ALTER TABLE users RENAME COLUMN school_email_verified_at TO email_verified_at;
ALTER TABLE users DROP COLUMN school_email, DROP COLUMN school;
ALTER TABLE email_verifications DROP COLUMN school_email, DROP COLUMN school;
