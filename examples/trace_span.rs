use tide::{Request, Response, StatusCode};
use tide_tracing_middleware::TracingMiddleware;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[async_std::main]
async fn main() -> tide::Result<()> {
    FmtSubscriber::builder().with_max_level(Level::DEBUG).init();

    let tracing_middleware = TracingMiddleware::new(
        "%t  %a(%{r}a)  %r(%M %U %Q %V) %s %b(bytes) %T(seconds) %D(milliseconds) REQ_HEADERS:%{ALL_REQ_HEADERS}xi RES_HEADERS:%{ALL_RES_HEADERS}xo",
    ).custom_request_replace("ALL_REQ_HEADERS", |req| {
        let pairs = req.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<String>>();
        "{".to_owned() + &pairs.join(",") + "}"
    }).custom_response_replace("ALL_RES_HEADERS", |res| {
        let pairs = res.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<String>>();
        "{".to_owned() + &pairs.join(",") + "}"
    }).gen_tracing_span(|_req| {
        tracing::info_span!("R", "{}", uuid::Uuid::new_v4().to_simple().to_string())
    });

    let mut app = tide::new();
    app.with(tracing_middleware);
    app.at("/index").get(index);
    app.listen("127.0.0.1:8080").await?;
    Ok(())
}

async fn index(_req: Request<()>) -> tide::Result {
    info!(a = "123", "index");
    let res = Response::builder(StatusCode::Ok)
        .body("hello world!")
        .build();
    Ok(res)
}
