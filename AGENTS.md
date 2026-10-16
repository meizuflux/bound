## Stack and structure

- Askama templates live in `templates/`
- `src/templates.rs` provides `HtmlTemplate<T>`, the local Salvo response adapter.
  Return `HtmlTemplate(template)` from HTML handlers. It sets the HTML content
  type and logs rendering failures before returning HTTP 500.
- Keep the local adapter; check Salvo version compatibility before replacing it
  with an external integration such as `askama_web`.
- SQLx provides a shared Postgres pool, currently limited to five connections.
  Startup connects using `DATABASE_URL`; `affix_state::inject` makes the pool
  available to handlers through `Depot`. Reuse that pool.
- Bun manages frontend dependencies and the Tailwind CLI
- `assets/css/app.css` imports Tailwind and scans `templates/`.
- Tailwind generates `public/css/app.css`; Salvo serves `public/` at `/public/`.
  Generated CSS, `node_modules/`, and `target/` are ignored by Git.

## Commands

All applicable commands exist in the `justfile`. Run them from the repository root.

CSS updates are served from disk; Askama template changes require rebuilding
and restarting Rust.

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
