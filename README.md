# tide-tracing-middleware
A middleware for tide using the tracing crate for logging.

迁移了 actix-web 自带的 log 中间件，以应用于 tide 框架。


## 开始使用

代码在 examples/basic.rs。

```rs
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
```

输出示例：

```
Sep 16 21:12:39.988  INFO tide_tracing_middleware: 127.0.0.1:56205 "GET /index?a=1&b=2 HTTP/1.1" 200 12 "-" "curl/7.64.1" 0.000278
```

## 自定义输出格式

下面的示例将输出大部分的信息，包括所有的请求头和响应头。

```rs
let tracing_middleware = TracingMiddleware::new(
	"%t  %a(%{r}a)  %r(%M %U %Q %V) %s %b(bytes) %T(seconds) %D(milliseconds) REQ_HEADERS:%{ALL_REQ_HEADERS}xi RES_HEADERS:%{ALL_RES_HEADERS}xo",
).custom_request_replace("ALL_REQ_HEADERS", |req| {
	let pairs = req.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<String>>();
	"{".to_owned() + &pairs.join(",") + "}"
}).custom_response_replace("ALL_RES_HEADERS", |res| {
	let pairs = res.iter().map(|(k, v)| format!("{}:{}", k, v)).collect::<Vec<String>>();
	"{".to_owned() + &pairs.join(",") + "}"
});
```

输出示例：

```
Sep 16 21:18:15.174  INFO tide_tracing_middleware: 2021-09-16T13:18:15  127.0.0.1:56234(127.0.0.1:56234)  GET /index?a=1&b=2 HTTP/1.1(GET /index a=1&b=2 HTTP/1.1) 200 12(bytes) 0.000437(seconds) 0.461000(milliseconds) REQ_HEADERS:{accept:["*/*"],user-agent:["curl/7.64.1"],host:["127.0.0.1:8080"]} RES_HEADERS:{content-type:["text/plain;charset=utf-8"]}
```

支持的标签和 actix-web 的 log 中间件一样，只是多添加了几个标签：

* `%M`: 请求的方法。
* `%V`: HTTP 的版本。
* `%Q`: 请求 URL 的查询参数。
* `%{FOO}xo`: 自定义响应标签。


## 生成 tracing span

下面的示例中使用 uuid 为每个请求生成一个 id，以便将该请求相关的日志关联起来。

```rs
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
```

输出示例：

```
Sep 16 21:22:29.564  INFO R{c7abce9aba3c4a2c9161c3df20a4141b}: trace_span: index a="123"
Sep 16 21:22:29.564  INFO R{c7abce9aba3c4a2c9161c3df20a4141b}: tide_tracing_middleware: 2021-09-16T13:22:29  127.0.0.1:56260(127.0.0.1:56260)  GET /index?a=1&b=2 HTTP/1.1(GET /index a=1&b=2 HTTP/1.1) 200 12(bytes) 0.000613(seconds) 0.626000(milliseconds) REQ_HEADERS:{user-agent:["curl/7.64.1"],accept:["*/*"],host:["127.0.0.1:8080"]} RES_HEADERS:{content-type:["text/plain;charset=utf-8"]}
```