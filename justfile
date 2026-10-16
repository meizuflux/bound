set dotenv-load

db_name := "bound-postgres"
db_volume := "bound-postgres-data"

default:
    @just --list

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
