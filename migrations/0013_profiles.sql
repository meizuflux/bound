ALTER TABLE users
    ADD COLUMN display_name TEXT NOT NULL DEFAULT '' CHECK (char_length(display_name) <= 80),
    ADD COLUMN phone_number TEXT NOT NULL DEFAULT '' CHECK (char_length(phone_number) <= 32),
    ADD COLUMN profile_visible BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN share_phone BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN profile_onboarding_done BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN profile_revision BIGINT NOT NULL DEFAULT 1,
    ADD CONSTRAINT shared_phone_requires_number CHECK (NOT share_phone OR phone_number <> '');
