ALTER TABLE users RENAME COLUMN email_verified_at TO school_email_verified_at;
ALTER TABLE users ADD COLUMN school_email TEXT UNIQUE;
ALTER TABLE users ADD COLUMN school TEXT;

-- Preserve existing Duke verifications while separating login and school emails.
UPDATE users SET school_email = email, school = 'duke'
WHERE split_part(email, '@', 2) = 'duke.edu';
UPDATE users SET school_email_verified_at = NULL WHERE school IS NULL;

ALTER TABLE users ADD CONSTRAINT verified_school_present CHECK (
    school_email_verified_at IS NULL OR (school_email IS NOT NULL AND school IS NOT NULL)
);

ALTER TABLE email_verifications ADD COLUMN school_email TEXT;
ALTER TABLE email_verifications ADD COLUMN school TEXT;
UPDATE email_verifications SET school_email = users.school_email, school = users.school
FROM users WHERE users.id = email_verifications.user_id;
DELETE FROM email_verifications WHERE school_email IS NULL;
ALTER TABLE email_verifications ALTER COLUMN school_email SET NOT NULL;
ALTER TABLE email_verifications ALTER COLUMN school SET NOT NULL;
