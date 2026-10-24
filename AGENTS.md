## Stack and structure

- Askama templates live in `templates/`
- `templates/layout.html` contains the shared page shell; `templates/index.html`
  extends it for the Bound landing page.
- Auth screens have focused templates in `templates/partials/auth/`: signup,
  signin, verification waiting/resend, link confirmation, link result, and
  unsupported legacy accounts. Credential fields and signout controls are shared
  includes; don't rebuild a global string-mode auth template.
- `src/auth/views.rs` owns the typed auth view contexts. The standalone `/account`
  page belongs to `src/account.rs` and `templates/partials/account.html`, not auth.
- `src/templates.rs` provides the shared `PageView` helper. HTMX requests render
  the requested panel directly; normal requests wrap that same panel with
  `templates/page.html` and the shared layout. The wrapper's `safe` filter
  receives only a typed, already-escaped Askama `Template`, never arbitrary HTML
  or user input. Keep rendering failures in `HtmlTemplate<T>`.
- `src/templates.rs` provides `HtmlTemplate<T>`, the local Salvo response adapter.
  Return `HtmlTemplate(template)` from HTML handlers. It sets the HTML content
  type and logs rendering failures before returning HTTP 500.
- Keep the local adapter; check Salvo version compatibility before replacing it
  with an external integration such as `askama_web`.
- SQLx provides a shared Postgres pool, currently limited to five connections.
  Startup connects using `DATABASE_URL`; `affix_state::inject` makes the pool
  available to handlers through `Depot`. Reuse that pool.
- Bun manages frontend dependencies and the Tailwind CLI
- HTMX 4 is installed through Bun and copied to `public/compiled/js/htmx.min.js` by the
  justfile. Use `hx-*` attributes in templates and return HTML fragments through
  `HtmlTemplate<T>` from Salvo routes. Shared fragments live in `templates/partials/`.
- In HTMX 4, inherited attributes need an explicit `:inherited` suffix; event
  names use the `htmx:phase:action` format.
- HTMX 4 sends `HX-Target` as `tag#id`; use `templates::htmx_target_id` when
  selecting a fragment response. Full/history restoration requests must render
  the complete page shell. `PageView` includes titles in partial responses;
  `public/js/app.js` moves focus for full-panel changes, never live results/polls.
- `assets/css/app.css` imports Tailwind and scans `templates/`.
- Tailwind generates `public/compiled/css/app.css`; Salvo serves `public/` at `/public/`.
  Generated assets belong in the ignored `public/compiled/` directory.

## Commands

All applicable commands exist in the `justfile`.

The backend runs through Bacon, which rebuilds and restarts on codebase changes. 
Tailwind watches CSS separately.

## Authentication

- `src/auth.rs` handles email/password sign-up, sign-in, verification, and sign-out.
  `/account` is a separate router/module protected by `auth::protect` and
  `auth::require_verified_user`; authenticated handlers read `CurrentUser` from
  Depot. Sign-in redirects to `/account`. Successful verification opens the skippable
  `/account/profile?onboarding=true` page until profile onboarding is saved or skipped,
  then redirects to `/account`.
- Passwords use salted Argon2id; opaque session tokens are stored as SHA-256
  hashes in Postgres and expire after 30 days. Cookies are HttpOnly and SameSite=Lax.
- `APP_URL` is the exact application origin (default `http://localhost:8000`),
  checked on POST requests; HTTPS enables Secure cookies.
- SQLx embeds migrations from `migrations/` and applies them at startup.
  `build.rs` tells Cargo to rebuild when the migration directory changes.
- Applied migrations are immutable, including comments and whitespace (SQLx checksums
  the entire file). Put subsequent schema changes in a new numbered migration.
- Sign-up and sign-in use one email: the user's school email. Reject sign-up
  unless its exact domain is mapped in `src/schools.rs`; do not accept arbitrary
  `.edu` addresses or subdomains. Currently only `duke.edu` is supported.
- Sign-up creates a session and immediately sends verification to that same email;
  there is no separate school-email onboarding form. Account and admin access
  require a supported `users.email` and `email_verified_at`. Derive the school
  from the email rather than storing a second email or school identifier.
- Migration 0005 makes existing verified school emails the login emails while
  preserving user IDs, passwords, roles, sessions, and verification timestamps.
  It aborts on conflicting login ownership or inconsistent verified mappings;
  resolve those explicitly rather than merging/deleting accounts. Legacy accounts
  without a verified school address are preserved, not promoted from pending
  email requests. Unsupported legacy logins must register with a school email.
- The account email is immutable. Future email-change support must invalidate
  outstanding verification tokens atomically or restore explicit email binding.
- Verification links expire after 30 minutes and are single-use. Store only
  token hashes alongside the intended user and expiry/cooldown timestamps. GET shows
  an HTMX form that submits on load; POST verifies without changing the email
  tab's session and says it can be closed. Resends replace the old link and
  have a one-minute cooldown.
- Auth forms use HTMX 4 with HTML fragments for updates and `HX-Redirect` for
  navigation. Keep normal form submissions working too.
- Signup errors are Askama-rendered inline alerts with field associations and
  `aria-invalid`. Preserve and escape the submitted email, never the password.
  Unsupported domains show the configured allowed domains. Return appropriate
  400/409/500 statuses; HTMX 4 swaps error fragments by default. Log backend
  failures but show only a safe, retryable form message to the user.
- The waiting page polls `/auth/verification/status` every two seconds using the
  current session. Pending polls return 204; verification redirects the original
  page to the account. Expired links stop polling and offer a resend to the same
  school email. Unverified users can sign out to use a different account.
- `src/mail.rs` sends through local Mailpit (SMTP `127.0.0.1:1025`, inbox
  `http://localhost:8025`). Use the justfile for its Apple container lifecycle.
  `SMTP_HOST`, `SMTP_PORT`, and `MAIL_FROM` have local defaults; no real email
  service or SMTP authentication/TLS is configured.
- Users default to `is_admin = false`; manage admin access by email through the
  justfile. The account must already exist; commands return the updated row.
- `/admin` uses `auth::require_admin`. Apply this hoop to future admin routers;
  it loads the current user and permissions from Postgres on each request and
  stores `CurrentUser` in Depot. Guests are redirected to sign-in; non-admins
  receive HTTP 403. Role changes apply to existing sessions immediately.
- `src/admin.rs` owns the admin router: `/admin` and `/admin/users`
  list users, `/admin/users/{id}` shows account details and recent activity, and
  `/admin/activity` lists activity.
  Queries live in `src/admin/data.rs`, typed views in `src/admin/views.rs`, and
  panels in `templates/partials/admin/`. Lists use email search and 25-row pages.
- Admin writes live in focused user/flight handlers and `src/admin/mutations.rs`.
  Use POST with `auth::protect` and `auth::require_admin`, typed confirmation for
  account deletion, and revision checks. User email remains immutable. Admin
  creation sends verification without changing the administrator's session.
  Suspension denies existing sessions and future sign-in. Preserve protections
  against removing one's own access or the last eligible administrator; serialize
  these checks and the mutation in one transaction.
- Migration 0006 starts durable `activity_events` tracking through user/session
  database triggers, including justfile role changes. No past events are backfilled.
  Migration 0008 adds actor/flight snapshots and detail fields. `src/audit.rs`
  binds actors with transaction-local settings and records domain changes within
  their mutation transaction. Older/direct-database events may lack attribution;
  never infer who made them. Do not log passwords or session/verification tokens.
  Activity history remains append-only through the application.

## Profiles

- `src/account/profile.rs` owns `/account/profile` and its POST save/skip routes;
  `templates/partials/account/profile.html` is shared between onboarding and account settings.
  Name and phone are optional; signup remains email/password only. Saving or skipping
  records `profile_onboarding_done` without blocking account or flight access.
- Migration 0013 adds profile fields. Profiles are visible by default; phone sharing
  defaults to false and requires explicit opt-in. Hiding a profile also hides its phone.
  Empty names display as “Fellow student”, never an email-derived fallback.
- `src/flights/travelers.rs` enforces viewer membership, school, verification, suspension,
  profile visibility, and active canonical flight checks in SQL. Only opted-in phone
  numbers enter the view. Detail, poll, and mutation responses use the same projection.
- Profile saves use optimistic revision checks and transaction-local audit attribution.
  Audit events record that preferences changed, never name or phone values.

## Shared flights

- `src/flights.rs` owns verified-student HTTP routes; typed views and focused
  panels live in `src/flights/views.rs` and `templates/partials/flights/`.
  `src/flights/data.rs` owns transactional domain operations shared with admin.
  `src/flights/catalog.rs` contains the explicitly supported airports/airlines.
- `/flights` searches shared flights, `/flights/new` reviews and creates a flight,
  `/flights/mine` lists the current user's trips, and `/flights/{id}` shows details.
  Search accepts a flight number or departure date independently; supplying both narrows results.
  Keep search fields outside the results swap target. Preserve normal forms and
  full GET pages alongside HTMX fragments. HTMX 4 `hx-disable` takes a selector
  such as `find button`, not a boolean.
- Flight IDs are permanent. Match normalized flight number, departure airport
  local date, and route; scheduled time is excluded from duplicate matching.
  Use the shared domain functions for concurrent creation, revision checks,
  correction approval, aliases, merges, and archive/restore. Never reassign
  memberships when correcting the schedule.
- Derive UTC from an airport's IANA time zone; reject nonexistent or ambiguous
  local times rather than assuming an offset. This is a curated manual catalog,
  not a live airline feed. Do not label community submissions as airline verified.
- User-flight memberships are unique. Joining is separate from confirming a
  particular revision. Corrections retain their base revision and are reviewed
  by admins; stale proposals must not overwrite newer details. Merges retain
  history and redirect old links. Student search views expose same-school counts. Flight details reveal visible profiles
  only to verified, unsuspended same-school members of that flight; never expose emails.

## Local Postgres

Postgres runs through Apple's `container` CLI.

- Container: `bound-postgres`, using the Postgres 18 image.
- Default address: `127.0.0.1:5432`; database and user: `bound`.
- Persistent volume: `bound-postgres-data`, mounted at `/var/lib/postgresql`.
- `.env` contains local credentials and must stay untracked. Never commit it or
  expose its credentials in output.
- Preserve database data unless deletion is explicitly requested.

## Behavior

- Update AGENTS.md when applicable.
