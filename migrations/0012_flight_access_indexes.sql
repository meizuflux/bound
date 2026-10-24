-- Index flight foreign keys for account deletion lookups.
CREATE INDEX flights_created_by ON flights (created_by) WHERE created_by IS NOT NULL;
CREATE INDEX flight_corrections_proposed_by ON flight_corrections (proposed_by) WHERE proposed_by IS NOT NULL;
CREATE INDEX flight_corrections_reviewed_by ON flight_corrections (reviewed_by) WHERE reviewed_by IS NOT NULL;
