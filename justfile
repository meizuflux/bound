set dotenv-load

db_name := "bound-postgres"
db_volume := "bound-postgres-data"

[private]
default:
    @just --list

# Run the backend and the Tailwind watcher together.
[parallel]
dev: run watch-css

# Watch, rebuild, and restart the Rust backend.
run: assets
    PATH="$HOME/.cargo/bin:$PATH" bacon --headless run-long

# Copy HTMX into the public assets.
assets:
    mkdir -p public/compiled/js
    cp node_modules/htmx.org/dist/htmx.min.js public/compiled/js/htmx.min.js

# Build and watch Tailwind CSS.
watch-css:
    bunx --bun @tailwindcss/cli -i ./assets/css/app.css -o ./public/compiled/css/app.css --watch

# Build minified CSS and the release binary.
build: assets
    bunx --bun @tailwindcss/cli -i ./assets/css/app.css -o ./public/compiled/css/app.css --minify
    cargo build --release

# Create the database container and volume once.
db-init:
    container system start --enable-kernel-install
    container volume create -s 5G '{{ db_volume }}'
    container create --name '{{ db_name }}' --memory 512M \
        --env POSTGRES_USER --env POSTGRES_PASSWORD --env POSTGRES_DB \
        --publish "127.0.0.1:$POSTGRES_PORT:5432" \
        --mount 'type=volume,source={{ db_volume }},target=/var/lib/postgresql' \
        docker.io/library/postgres:18

# Start Postgres.
db-up:
    container start '{{ db_name }}'

# Stop Postgres, preserving data.
db-down:
    container stop '{{ db_name }}'

# Follow Postgres logs.
db-logs:
    container logs --follow '{{ db_name }}'

# Open psql, optionally passing arguments.
[positional-arguments]
db-psql *args:
    container exec -it '{{ db_name }}' psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" "$@"

# Grant admin access to an existing account by email.
[positional-arguments]
admin-grant email:
    @printf '%s\n' "UPDATE users SET is_admin = true WHERE email = lower(btrim(:'email')) RETURNING email, is_admin;" | container exec -i '{{ db_name }}' psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" --set=ON_ERROR_STOP=1 --set=email="$1"

# Revoke admin access by email.
[positional-arguments]
admin-revoke email:
    @printf '%s\n' "UPDATE users SET is_admin = false WHERE email = lower(btrim(:'email')) RETURNING email, is_admin;" | container exec -i '{{ db_name }}' psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" --set=ON_ERROR_STOP=1 --set=email="$1"
