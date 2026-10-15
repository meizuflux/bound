mod templates;

use askama::Template;
use salvo::prelude::*;
use salvo::serve_static::StaticDir;

use templates::HtmlTemplate;

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    name: &'a str,
}

#[handler]
async fn hello() -> HtmlTemplate<IndexTemplate<'static>> {
    HtmlTemplate(IndexTemplate { name: "world" })
}

#[tokio::main]
async fn main() {
    // Initialize logging subsystem
    tracing_subscriber::fmt().init();

    // Bind server to port 8000
    let acceptor = TcpListener::new("0.0.0.0:8000").bind().await;

    let router = Router::new()
        .push(Router::with_path("/").get(hello))
        .push(Router::with_path("public/{**path}").get(StaticDir::new(["public"])));

    // Print router structure for debugging
    println!("{router:?}");

    // Start serving requests
    Server::new(acceptor).serve(router).await;
}
