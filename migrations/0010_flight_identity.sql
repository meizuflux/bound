-- A merged row is retained as a redirect and historical evidence. It must not
-- reserve the original identity after the destination's route/date is corrected.
-- Archived canonical rows still reserve their identity until an admin restores
-- them, preventing students from accidentally recreating moderated removals.
DO $$
DECLARE identity_constraint TEXT;
BEGIN
    SELECT conname INTO STRICT identity_constraint
    FROM pg_constraint
    WHERE conrelid = 'flights'::regclass AND contype = 'u';
    EXECUTE format('ALTER TABLE flights DROP CONSTRAINT %I', identity_constraint);
END;
$$;

CREATE UNIQUE INDEX flights_canonical_identity
ON flights (flight_number, departure_date, origin, destination, instance)
WHERE merged_into IS NULL;
