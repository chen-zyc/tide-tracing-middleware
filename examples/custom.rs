use tide::{Request, Response, StatusCode};
use tide_tracing_middleware::TracingMiddleware;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[async_std::main]
async fn main() -> tide::Result<()> {
    FmtSubscriber::builder().with_max_level(Level::DEBUG).init();

    let tracing_middleware = TracingMiddleware::new(
        "%t  %a(%{r}a)  %r(%M %U %Q %V) %s %b(bytes) %T(seconds) %D(milliseconds) %{ALL_REQ_HEADERS}xi",
    );
    let tracing_middleware = tracing_middleware.custom_request_replace("ALL_REQ_HEADERS", |req| {
        let mut header_pair = vec![];
        for (header_name, header_values) in req {
            header_pair.push(format!("{}:{}", header_name.as_str(), header_values));
        }
        "{".to_owned() + &header_pair.join(",") + "}"
    });

    let mut app = tide::new();
    app.with(tracing_middleware);
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
