-- Foreign-key lookups during account deletion and session revocation should not
-- scan every historical event or expired session as the application grows.
CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE INDEX activity_events_actor_id ON activity_events (actor_id) WHERE actor_id IS NOT NULL;
CREATE INDEX flights_created_by ON flights (created_by) WHERE created_by IS NOT NULL;
CREATE INDEX flight_corrections_proposed_by ON flight_corrections (proposed_by) WHERE proposed_by IS NOT NULL;
CREATE INDEX flight_corrections_reviewed_by ON flight_corrections (reviewed_by) WHERE reviewed_by IS NOT NULL;
