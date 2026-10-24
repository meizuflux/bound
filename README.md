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

## Auth templates

Each auth screen has its own [panel template](<templates/partials/auth/signup.html>)
and typed context in [auth views](<src/auth/views.rs>). HTMX responses render only
that panel; regular requests wrap it with the [shared page shell](<templates/page.html>)
and layout through the [page renderer](<src/templates.rs>). There is no global
mode selector or template-block slicing.

The account page is a separate feature at `/account`, owned by the
[account module](<src/account.rs>) and [account panel](<templates/partials/account.html>).
It requires a valid session and verified school email; sign-in and verification
redirect there. Account UI does not live under `/auth`.

Only genuinely reused markup is shared: [credential fields](<templates/partials/auth/credential_fields.html>)
and [signout controls](<templates/partials/auth/signout.html>). Waiting, expired,
and delivery-failure states stay together in the [verification panel](<templates/partials/auth/verification.html>).

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

## Admin

Verified admins can open `/admin` from their account page. It includes user
creation, role and access management, session revocation, confirmed account
deletion, flight creation/editing/archive/restore, alternate flight numbers,
correction review, and duplicate merging. Account emails remain immutable;
new accounts must verify their school email. Activity history is retained as an
audit trail. Admins cannot delete or remove access from their own account or
remove the last eligible administrator.
Grant or revoke access for an existing account with `just admin-grant EMAIL` or
`just admin-revoke EMAIL`.

Activity tracking begins with migration 0006. Database triggers record account
creation/deletion, verification, email and role changes, and session starts/ends,
including role changes made through the local commands. Existing history is not
backfilled. Migration 0008 adds actor snapshots, flight references, and change
details for application mutations. Older events and direct database changes may
have no known actor; the interface labels these honestly. Times display in UTC.
Passwords and tokens are never included.

## Shared flights

Verified students can find flights at `/flights`, add a missing flight after a
review step, and manage their trips at `/flights/mine`. Search updates results
in place with HTMX, while ordinary links and form submissions remain supported.
Each shared flight has a permanent ID; departure time is a correctable detail,
not part of duplicate matching. Search results show only counts from the viewer's school;
flight details show visible traveler names to verified same-school students who have
joined that flight. Email addresses are never exposed to other travelers.

The first version uses a curated US airport and airline catalog in
`src/flights/catalog.rs`, not a live airline schedule feed. Times are explicitly
local to the departure airport and validated against its IANA time zone. Invalid
or ambiguous daylight-saving times are rejected rather than guessed. Flights
are community-submitted; check the airline for current travel information.

Travelers can explicitly confirm a flight revision or propose corrections after
joining. Admin review applies approved changes atomically, rejecting stale edits.
Creating a flight implicitly confirms its initial details, while the public count
starts at zero and counts other travelers. Creators can confirm later revisions.
Joining an existing flight alone never confirms the schedule. Duplicate merges preserve memberships
and old links; flight deletion is an archive with restoration available.

## Profiles

After email verification, students can complete their profile or skip straight to
their account. Name and phone number are optional; both can be edited from the
account page at any time. Signup remains email and password only.

Joining a flight makes your profile visible by default to verified students from
your school who also joined it. An empty name appears as “Fellow student”. Profile
settings allow hiding your profile across all flights. Phone numbers remain private
unless you separately opt to share them with fellow travelers; hiding a profile
also hides its phone. Phone numbers are self-reported, not verified.
