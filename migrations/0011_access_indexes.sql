-- Foreign-key lookups during account deletion and session revocation should not
-- scan every historical event or expired session as the application grows.
CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE INDEX activity_events_actor_id ON activity_events (actor_id) WHERE actor_id IS NOT NULL;
