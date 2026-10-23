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
  Depot. Sign-in and successful verification polling redirect to `/account`.
- Passwords use salted Argon2id; opaque session tokens are stored as SHA-256
  hashes in Postgres and expire after 30 days. Cookies are HttpOnly and SameSite=Lax.
- `APP_URL` is the exact application origin (default `http://localhost:8000`),
  checked on POST requests; HTTPS enables Secure cookies.
- SQLx embeds migrations from `migrations/` and applies them at startup.
  `build.rs` tells Cargo to rebuild when the migration directory changes.
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
- `src/admin.rs` owns the read-only admin router: `/admin` and `/admin/users`
  list users, `/admin/users/{id}` shows account details and recent activity, and
  `/admin/activity` lists activity.
  Queries live in `src/admin/data.rs`, typed views in `src/admin/views.rs`, and
  panels in `templates/partials/admin/`. Lists use email search and 25-row pages.
- Migration 0006 starts durable `activity_events` tracking through user/session
  database triggers, including justfile role changes. No past events are backfilled.
  Events describe the affected account, not the actor; never infer who made a
  change. Do not log passwords, session tokens, or verification tokens. Future
  admin writes must keep `auth::protect` and `auth::require_admin` and explicitly
  record the actor if attribution is needed.

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
