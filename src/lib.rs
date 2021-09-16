use std::collections::HashSet;
use std::convert::TryFrom;
use std::fmt::{self, Display, Error as fmtError, Formatter, Result as fmtResult};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::AsyncRead;
use pin_project::{pin_project, pinned_drop};
use regex::{Regex, RegexSet};
use tide::http::headers::HeaderName;
use tide::{Body, Middleware, Next, Request, Response};
use time::OffsetDateTime;
use tracing::{error, info, Span};
use tracing_futures::Instrument;

/// `TracingMiddleware` for logging request and response info to the terminal.
///
/// ## Usage
///
/// Create `TracingMiddleware` middleware with the specified `format`.
/// Default `TracingMiddleware` could be created with `default` method, it uses the
/// default format:
///
/// ```plain
/// %a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T
/// ```
///
/// ```rust
/// use tide::{Request, Response, StatusCode};
// use tide_tracing_middleware::TracingMiddleware;
// use tracing::Level;
// use tracing_subscriber::FmtSubscriber;
//
// #[async_std::main]
// async fn main() -> tide::Result<()> {
//     FmtSubscriber::builder().with_max_level(Level::DEBUG).init();
//
//     let mut app = tide::new();
//     app.with(TracingMiddleware::default());
//     app.at("/index").get(index);
//     app.listen("127.0.0.1:8080").await?;
//     Ok(())
// }
//
// async fn index(_req: Request<()>) -> tide::Result {
//     let res = Response::builder(StatusCode::Ok)
//         .body("hello world!")
//         .build();
//     Ok(res)
// }
///
/// ## Format
///
/// `%%`  The percent sign
///
/// `%a`  Remote IP-address (IP-address of proxy if using reverse proxy)
///
/// `%t`  Time when the request was started to process (in rfc3339 format)
///
/// `%r`  First line of request
///
/// `%s`  Response status code
///
/// `%b`  Size of response body in bytes, not including HTTP headers
///
/// `%T`  Time taken to serve the request, in seconds with floating fraction in .06f format
///
/// `%D`  Time taken to serve the request, in milliseconds
///
/// `%U`  Request URL
///
/// `%M`  Request method
///
/// `%V`  Request HTTP version
///
/// `%Q`  Request URL's query string
///
/// `%{r}a`  Real IP remote address **\***
///
/// `%{FOO}i`  request.headers['FOO']
///
/// `%{FOO}o`  response.headers['FOO']
///
/// `%{FOO}e`  os.environ['FOO']
///
/// `%{FOO}xi`  [custom request replacement](TracingMiddleware::custom_request_replace) labelled "FOO"
///
/// `%{FOO}xo`  [custom response replacement](TracingMiddleware::custom_response_replace) labelled "FOO"
///
pub struct TracingMiddleware<State: Clone + Send + Sync + 'static> {
    inner: Arc<Inner<State>>,
}

struct Inner<State: Clone + Send + Sync + 'static> {
    format: Format<State>,
    exclude: HashSet<String>,
    exclude_regex: RegexSet,
    gen_tracing_span: Option<fn(&Request<State>) -> Span>,
}

impl<State> TracingMiddleware<State>
where
    State: Clone + Send + Sync + 'static,
{
    /// Create `TracingMiddleware` middleware with the specified `format`.
    pub fn new(s: &str) -> Self {
        Self {
            inner: Arc::new(Inner {
                format: Format::new(s),
                exclude: HashSet::new(),
                exclude_regex: RegexSet::empty(),
                gen_tracing_span: None,
            }),
        }
    }

    /// Ignore and do not log access info for specified path.
    pub fn exclude<T: Into<String>>(mut self, path: T) -> Self {
        Arc::get_mut(&mut self.inner)
            .unwrap()
            .exclude
            .insert(path.into());
        self
    }

    /// Ignore and do not log access info for paths that match regex
    pub fn exclude_regex<T: Into<String>>(mut self, path: T) -> Self {
        let inner = Arc::get_mut(&mut self.inner).unwrap();
        let mut patterns = inner.exclude_regex.patterns().to_vec();
        patterns.push(path.into());
        let regex_set = RegexSet::new(patterns).unwrap();
        inner.exclude_regex = regex_set;
        self
    }

    /// Register a function that receives a Request and returns a String for use in the
    /// log line. The label passed as the first argument should match a replacement substring in
    /// the logger format like `%{label}xi`.
    ///
    /// It is convention to print "-" to indicate no output instead of an empty string.
    pub fn custom_request_replace(
        mut self,
        label: &str,
        f: impl Fn(&Request<State>) -> String + Send + Sync + 'static,
    ) -> Self {
        let inner = Arc::get_mut(&mut self.inner).unwrap();

        let ft = inner.format.0.iter_mut().find(
            |ft| matches!(ft, FormatText::CustomRequest(unit_label, _) if label == unit_label),
        );

        if let Some(FormatText::CustomRequest(_, request_fn)) = ft {
            // replace into None or previously registered fn using same label
            request_fn.replace(CustomRequestFn {
                inner_fn: Arc::new(f),
            });
        } else {
            // non-printed request replacement function diagnostic
            error!(
                "Attempted to register custom request logging function for nonexistent label: {}",
                label
            );
        }

        self
    }

    /// Register a function that receives a Response and returns a String for use in the
    /// log line. The label passed as the first argument should match a replacement substring in
    /// the logger format like `%{label}xo`.
    ///
    /// It is convention to print "-" to indicate no output instead of an empty string.
    pub fn custom_response_replace(
        mut self,
        label: &str,
        f: impl Fn(&Response) -> String + Send + Sync + 'static,
    ) -> Self {
        let inner = Arc::get_mut(&mut self.inner).unwrap();

        let ft = inner.format.0.iter_mut().find(
            |ft| matches!(ft, FormatText::CustomResponse(unit_label, _) if label == unit_label),
        );

        if let Some(FormatText::CustomResponse(_, response_fn)) = ft {
            // replace into None or previously registered fn using same label
            response_fn.replace(CustomResponseFn {
                inner_fn: Arc::new(f),
            });
        } else {
            // non-printed response replacement function diagnostic
            error!(
                "Attempted to register custom response logging function for nonexistent label: {}",
                label
            );
        }

        self
    }

    pub fn gen_tracing_span(mut self, f: fn(&Request<State>) -> Span) -> Self {
        let inner = Arc::get_mut(&mut self.inner).unwrap();
        inner.gen_tracing_span.replace(f);
        self
    }
}

impl<State: Clone + Send + Sync + 'static> Default for TracingMiddleware<State> {
    /// Create `TracingMiddleware` middleware with format:
    ///
    /// ```ignore
    /// %a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T
    /// ```
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                format: Format::default(),
                exclude: HashSet::new(),
                exclude_regex: RegexSet::empty(),
                gen_tracing_span: None,
            }),
        }
    }
}

#[tide::utils::async_trait]
impl<State> Middleware<State> for TracingMiddleware<State>
where
    State: Clone + Send + Sync + 'static,
{
    async fn handle(&self, request: Request<State>, next: Next<'_, State>) -> tide::Result {
        let path = request.url().path();
        if self.inner.exclude.contains(path) || self.inner.exclude_regex.is_match(path) {
            return Ok(next.run(request).await);
        }

        let now = OffsetDateTime::now_utc();
        let mut format = self.inner.format.clone();
        for unit in &mut format.0 {
            unit.render_request(now, &request);
        }

        let span = if let Some(f) = self.inner.gen_tracing_span.as_ref() {
            f(&request)
        } else {
            Span::none()
        };
        let cloned_span = span.clone();

        let mut resp = next.run(request).instrument(span).await;

        for unit in &mut format.0 {
            unit.render_response(&resp);
        }

        let body = resp.take_body();
        let body_len = body.len();
        let body_mime = body.mime().clone();
        let mut new_body = Body::from_reader(
            futures::io::BufReader::new(StreamLog {
                body,
                format,
                size: 0,
                time: now,
                span: cloned_span,
            }),
            body_len,
        );
        new_body.set_mime(body_mime);

        resp.set_body(new_body);
        Ok(resp)
    }
}

#[doc(hidden)]
#[derive(Debug, Clone)]
struct Format<State: Clone + Send + Sync + 'static>(Vec<FormatText<State>>);

impl<State: Clone + Send + Sync + 'static> Format<State> {
    /// Create a `Format` from a format string.
    ///
    /// Returns `None` if the format string syntax is incorrect.
    fn new(s: &str) -> Format<State> {
        let fmt = Regex::new(r"%(\{([A-Za-z0-9\-_]+)\}([aioe]|xi|xo)|[atPrUsbTDMVQ]?)").unwrap();

        let mut idx = 0;
        let mut results = Vec::new();
        for cap in fmt.captures_iter(s) {
            let m = cap.get(0).unwrap();
            let pos = m.start();
            if idx != pos {
                results.push(FormatText::Str(s[idx..pos].to_owned()));
            }
            idx = m.end();

            if let Some(key) = cap.get(2) {
                results.push(match cap.get(3).unwrap().as_str() {
                    "a" => {
                        if key.as_str() == "r" {
                            FormatText::RealIPRemoteAddr
                        } else {
                            unreachable!()
                        }
                    }
                    "i" => FormatText::RequestHeader(HeaderName::try_from(key.as_str()).unwrap()),
                    "o" => FormatText::ResponseHeader(HeaderName::try_from(key.as_str()).unwrap()),
                    "e" => FormatText::EnvironHeader(key.as_str().to_owned()),
                    "xi" => FormatText::CustomRequest(key.as_str().to_owned(), None),
                    "xo" => FormatText::CustomResponse(key.as_str().to_owned(), None),
                    _ => unreachable!(),
                })
            } else {
                let m = cap.get(1).unwrap();
                results.push(match m.as_str() {
                    "%" => FormatText::Percent,
                    "a" => FormatText::RemoteAddr,
                    "t" => FormatText::RequestTime,
                    "r" => FormatText::RequestLine,
                    "s" => FormatText::ResponseStatus,
                    "b" => FormatText::ResponseSize,
                    "M" => FormatText::Method,
                    "V" => FormatText::Version,
                    "Q" => FormatText::Query,
                    "U" => FormatText::UrlPath,
                    "T" => FormatText::Time,
                    "D" => FormatText::TimeMillis,
                    _ => FormatText::Str(m.as_str().to_owned()),
                });
            }
        }
        if idx != s.len() {
            results.push(FormatText::Str(s[idx..].to_owned()));
        }

        Format(results)
    }
}

impl<State: Clone + Send + Sync + 'static> Default for Format<State> {
    /// Return the default formatting style for the `TracingMiddleware`:
    fn default() -> Self {
        Format::new(r#"%a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T"#)
    }
}

/// A string of text to be logged. This is either one of the data
/// fields supported by the `TracingMiddleware`, or a custom `String`.
#[doc(hidden)]
#[non_exhaustive]
#[derive(Debug, Clone)]
enum FormatText<State: Clone + Send + Sync + 'static> {
    Str(String),
    Percent,
    RequestLine,
    RequestTime,
    ResponseStatus,
    ResponseSize,
    Time,
    TimeMillis,
    RemoteAddr,
    RealIPRemoteAddr,
    Method,
    Version,
    UrlPath,
    Query,
    RequestHeader(HeaderName),
    ResponseHeader(HeaderName),
    EnvironHeader(String),
    CustomRequest(String, Option<CustomRequestFn<State>>),
    CustomResponse(String, Option<CustomResponseFn>),
}

#[doc(hidden)]
#[derive(Clone)]
pub struct CustomRequestFn<State: Clone + Send + Sync + 'static> {
    inner_fn: Arc<dyn Fn(&Request<State>) -> String + Sync + Send>,
}

impl<State> CustomRequestFn<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn call(&self, req: &Request<State>) -> String {
        (self.inner_fn)(req)
    }
}

impl<State> fmt::Debug for CustomRequestFn<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmtResult {
        f.write_str("custom_request_fn")
    }
}

#[doc(hidden)]
#[derive(Clone)]
pub struct CustomResponseFn {
    inner_fn: Arc<dyn Fn(&Response) -> String + Sync + Send>,
}

impl CustomResponseFn {
    fn call(&self, resp: &Response) -> String {
        (self.inner_fn)(resp)
    }
}

impl fmt::Debug for CustomResponseFn {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmtResult {
        f.write_str("custom_response_fn")
    }
}

impl<State> FormatText<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn render_request(&mut self, now: OffsetDateTime, req: &Request<State>) {
        match &*self {
            FormatText::RequestLine => {
                *self = if let Some(query_str) = req.url().query() {
                    FormatText::Str(format!(
                        "{} {}?{} {}",
                        req.method(),
                        req.url().path(),
                        query_str,
                        req.version().as_ref().map_or("?", |v| v.as_ref())
                    ))
                } else {
                    FormatText::Str(format!(
                        "{} {} {}",
                        req.method(),
                        req.url().path(),
                        req.version().as_ref().map_or("?", |v| v.as_ref())
                    ))
                };
            }
            FormatText::Method => *self = FormatText::Str(req.method().to_string()),
            FormatText::Version => {
                *self = FormatText::Str(
                    req.version()
                        .as_ref()
                        .map_or("?".to_owned(), |v| v.to_string()),
                )
            }
            FormatText::Query => {
                *self = FormatText::Str(req.url().query().map_or("-".to_owned(), |v| v.to_string()))
            }
            FormatText::UrlPath => *self = FormatText::Str(req.url().path().to_string()),
            FormatText::RequestTime => *self = FormatText::Str(now.format("%Y-%m-%dT%H:%M:%S")),
            FormatText::RequestHeader(ref name) => {
                let s = if let Some(val) = req.header(name) {
                    if let Some(v) = val.get(0) {
                        v.as_str()
                    } else {
                        "_"
                    }
                } else {
                    "-"
                };
                *self = FormatText::Str(s.to_string());
            }
            FormatText::RemoteAddr => {
                *self = if let Some(addr) = req.remote() {
                    FormatText::Str(addr.to_string())
                } else {
                    FormatText::Str("-".to_string())
                };
            }
            FormatText::RealIPRemoteAddr => {
                *self = if let Some(remote) = req.peer_addr() {
                    FormatText::Str(remote.to_string())
                } else {
                    FormatText::Str("-".to_string())
                };
            }
            FormatText::CustomRequest(_, request_fn) => {
                *self = match request_fn {
                    Some(f) => FormatText::Str(f.call(req)),
                    None => FormatText::Str("-".to_owned()),
                };
            }
            _ => (),
        }
    }

    fn render_response(&mut self, resp: &Response) {
        match &*self {
            FormatText::ResponseStatus => {
                *self = FormatText::Str(format!("{}", resp.status() as u16))
            }
            FormatText::ResponseHeader(name) => {
                let s = if let Some(val) = resp.header(name) {
                    if let Some(v) = val.get(0) {
                        v.as_str()
                    } else {
                        "-"
                    }
                } else {
                    "-"
                };
                *self = FormatText::Str(s.to_string())
            }
            FormatText::CustomResponse(_, response_fn) => {
                *self = match response_fn {
                    Some(f) => FormatText::Str(f.call(resp)),
                    None => FormatText::Str("-".to_owned()),
                };
            }
            _ => (),
        }
    }

    fn render(
        &self,
        fmt: &mut Formatter<'_>,
        size: usize,
        entry_time: OffsetDateTime,
    ) -> Result<(), fmtError> {
        match *self {
            FormatText::Str(ref string) => fmt.write_str(string),
            FormatText::Percent => "%".fmt(fmt),
            FormatText::ResponseSize => size.fmt(fmt),
            FormatText::Time => {
                let rt = OffsetDateTime::now_utc() - entry_time;
                let rt = rt.as_seconds_f64();
                fmt.write_fmt(format_args!("{:.6}", rt))
            }
            FormatText::TimeMillis => {
                let rt = OffsetDateTime::now_utc() - entry_time;
                let rt = (rt.whole_nanoseconds() as f64) / 1_000_000.0;
                fmt.write_fmt(format_args!("{:.6}", rt))
            }
            FormatText::EnvironHeader(ref name) => {
                if let Ok(val) = std::env::var(name) {
                    fmt.write_fmt(format_args!("{}", val))
                } else {
                    "-".fmt(fmt)
                }
            }
            _ => Ok(()),
        }
    }
}

#[pin_project(PinnedDrop)]
struct StreamLog<State: Clone + Send + Sync + 'static> {
    #[pin]
    body: Body,
    format: Format<State>,
    size: usize,
    time: OffsetDateTime,
    span: Span,
}

#[pinned_drop]
impl<State: Clone + Send + Sync + 'static> PinnedDrop for StreamLog<State> {
    fn drop(self: Pin<&mut Self>) {
        let render = |fmt: &mut Formatter<'_>| {
            for unit in &self.format.0 {
                unit.render(fmt, self.size, self.time)?;
            }
            Ok(())
        };
        info!(parent: &self.span, "{}", FormatDisplay(&render));
    }
}

impl<State> AsyncRead for StreamLog<State>
where
    State: Clone + Send + Sync + 'static,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.project();
        let res = this.body.poll_read(cx, buf);
        if let Poll::Ready(size) = &res {
            *this.size += if let Ok(n) = size { *n } else { 0 };
        }
        res
    }
}

/// Converter to get a String from something that writes to a Formatter.
struct FormatDisplay<'a>(&'a dyn Fn(&mut Formatter<'_>) -> Result<(), fmtError>);

impl<'a> Display for FormatDisplay<'a> {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), fmtError> {
        (self.0)(fmt)
    }
}
