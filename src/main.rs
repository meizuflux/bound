mod templates;

use askama::Template;
use salvo::prelude::*;
use salvo::serve_static::StaticDir;
use sqlx::postgres::PgPoolOptions;

use templates::HtmlTemplate;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate;

#[handler]
async fn index() -> HtmlTemplate<IndexTemplate> {
    HtmlTemplate(IndexTemplate)
}

fn main() -> Result<(), dotenvy::Error> {
    // Load local configuration before starting Tokio's worker threads.
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(error) if error.not_found() => {}
        Err(error) => return Err(error),
    }
    run();
    Ok(())
}

#[tokio::main]
async fn run() {
    // Initialize logging subsystem
    tracing_subscriber::fmt().init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is not set");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");
    tracing::info!("Connected to Postgres");

    // Bind server to port 8000
    let acceptor = TcpListener::new("0.0.0.0:8000").bind().await;

    let router = Router::new()
        .hoop(affix_state::inject(pool))
        .get(index)
        .push(Router::with_path("public/{**path}").get(StaticDir::new(["public"])));

    // Print router structure for debugging
    println!("{router:?}");

    // Start serving requests
    Server::new(acceptor).serve(router).await;
}
