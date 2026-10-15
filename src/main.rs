use askama::Template;
use salvo::prelude::*;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    name: &'a str,
}

#[handler]
async fn hello() -> Result<Text<String>, salvo::Error> {
    let template = IndexTemplate { name: "world" };

    Ok(Text::Html(template.render().unwrap()))
}

#[tokio::main]
async fn main() {
    // Initialize logging subsystem
    tracing_subscriber::fmt().init();

    // Bind server to port 8000
    let acceptor = TcpListener::new("0.0.0.0:8000").bind().await;

    // Render the greeting at /.
    let router = Router::new().get(hello);

    // Print router structure for debugging
    println!("{router:?}");

    // Start serving requests
    Server::new(acceptor).serve(router).await;
}
