use tide::{Request, Response, StatusCode};
use tide_tracing_middleware::TracingMiddleware;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[async_std::main]
async fn main() -> tide::Result<()> {
    FmtSubscriber::builder().with_max_level(Level::DEBUG).init();

    let mut app = tide::new();
    app.with(TracingMiddleware::default());
    app.at("/index").get(index);
    app.listen("127.0.0.1:8080").await?;
    Ok(())
}

async fn index(_req: Request<()>) -> tide::Result {
    let res = Response::builder(StatusCode::Ok)
        .body("hello world!")
        .build();
    Ok(res)
}
