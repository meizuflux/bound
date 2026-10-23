# Bound

Find other college students on your flight.

## School accounts

Sign up and sign in with your school email and password. Signup accepts only exact
known domains from the [school domain map](<src/schools.rs>) (currently `duke.edu`),
not arbitrary `.edu` addresses or subdomains. Verification is sent immediately to
that same address; there is no second email-entry step. Account and admin access
require email verification.

Verification links last 30 minutes. The waiting page updates automatically after
verification, and offers resending with a one-minute cooldown. Failed delivery
and expired links can be retried without creating another account.

## Existing accounts

The [single-email migration](<migrations/0005_single_school_email.sql>) runs at
startup. Existing verified users sign in with their verified school email instead
of their former personal email; passwords, roles, sessions, and user IDs remain
unchanged. Unverified school-email requests are never promoted into login emails.
Unsupported legacy accounts and their data are retained, but cannot access the app;
register with a supported school email instead.

The migration stops rather than merging accounts if a verified school email is
already another account's login, or a verified legacy school mapping is
inconsistent. Resolve those conflicts explicitly before retrying startup. Stop
old application processes before deploying the incompatible schema change.

Run tests with `just test` with local Postgres running.
