## Stack and structure

- Askama templates live in `templates/`
- `templates/layout.html` contains the shared page shell; `templates/index.html`
  extends it for the Bound landing page.
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

- `src/auth.rs` handles email/password sign-up, sign-in, account, and sign-out.
- Passwords use salted Argon2id; opaque session tokens are stored as SHA-256
  hashes in Postgres and expire after 30 days. Cookies are HttpOnly and SameSite=Lax.
- `APP_URL` is the exact application origin (default `http://localhost:8000`),
  checked on POST requests; HTTPS enables Secure cookies.
- SQLx embeds migrations from `migrations/` and applies them at startup.
  `build.rs` tells Cargo to rebuild when the migration directory changes.
- Email verification and university email restrictions are not implemented yet.
- Users default to `is_admin = false`; manage admin access by email through the
  justfile. The account must already exist; commands return the updated row.
- `/admin` uses `auth::require_admin`. Apply this hoop to future admin routers;
  it loads the current user and permissions from Postgres on each request and
  stores `CurrentUser` in Depot. Guests are redirected to sign-in; non-admins
  receive HTTP 403. Role changes apply to existing sessions immediately.

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
