//! Redirects. A 301, 302, 303, 307 or 308 with a usable `Location` is followed, up to 20
//! times, each hop a separate `Transport::execute` call: 303 becomes GET without content,
//! 301 and 302 turn POST into GET without content, and otherwise the method and content
//! are repeated. Credentials stay on the original origin. An https to http hop is not
//! followed, and that 3xx is the final response. More redirects than the limit, an
//! unusable `Location`, incomplete header evidence and an opaque redirect complete
//! unsuccessfully.
use dynamic_openapi_client::{
    Cancellation, Diagnostic, Header, Outcome,
    diagnostics::Code,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding};
use openbindings_openapi::{
    KIND_30, KIND_31, OpenApi, Options, SECURITY_POINT_30, SECURITY_POINT_31,
};
use openbindings_source_inspection::CancellationToken;
use serde_json_compat::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::Arc,
};

const KINDS: [(&str, &str, &str); 2] = [
    (KIND_31, "3.1.2", SECURITY_POINT_31),
    (KIND_30, "3.0.4", SECURITY_POINT_30),
];
const ORIGIN: &str = "https://api.example.test";
const FAILED: &str = "ERR_EXECUTION_FAILED";
const OK: &str = r#"{"ok":true}"#;

fn source(kind: &str, version: &str) -> Source {
    let ok = json!({"200": {"description": "ok"}});
    let body = json!({"content": {"application/json": {}}});
    let document = json!({
        "openapi": version,
        "info": {"title": "Redirects", "version": "1"},
        "servers": [{"url": ORIGIN}],
        "components": {"securitySchemes": {
            "Token": {"type": "http", "scheme": "bearer"},
            "Key": {"type": "apiKey", "in": "header", "name": "X-Key"}
        }},
        "paths": {
            "/items": {
                "get": {"responses": ok},
                "post": {"requestBody": body, "responses": ok},
                "put": {"requestBody": body, "responses": ok},
                "patch": {"requestBody": body, "responses": ok},
                "delete": {"responses": ok}
            },
            "/token": {"get": {"security": [{"Token": []}], "responses": ok}},
            "/key": {"get": {"security": [{"Key": []}], "responses": ok}}
        }
    });
    Source {
        kind: kind.into(),
        content: Some(
            JsonValue::parse(json!({"document": document.to_string()}).to_string()).unwrap(),
        ),
        ..Default::default()
    }
}

/// One request as the host saw it.
#[derive(Clone, Debug, PartialEq)]
struct Hop {
    method: String,
    target: String,
    /// Lowercased names with their values, in order.
    headers: Vec<(String, String)>,
    body: Option<String>,
}
impl Hop {
    fn of(request: &PreparedRequest) -> Self {
        Self {
            method: request.method().as_str().to_owned(),
            target: request.target().to_owned(),
            headers: request
                .headers()
                .iter()
                .map(|h| {
                    (
                        h.name().to_ascii_lowercase(),
                        String::from_utf8_lossy(h.value()).into_owned(),
                    )
                })
                .collect(),
            body: request
                .body()
                .map(|b| String::from_utf8(b.to_vec()).unwrap()),
        }
    }
    fn header(&self, name: &str) -> Option<&str> {
        let mut values = self.headers.iter().filter(|(n, _)| n == name);
        let value = values.next().map(|(_, v)| v.as_str());
        assert!(values.next().is_none(), "one {name} header at most");
        value
    }
}

/// One response as a host delivers it.
#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<Header>,
    body: &'static str,
    delivery: HeaderDelivery,
    opaque: bool,
}
impl Reply {
    fn status(status: u16) -> Self {
        Self {
            status,
            headers: vec![],
            body: "",
            delivery: HeaderDelivery::Complete,
            opaque: false,
        }
    }
    /// A 200 with a JSON value.
    fn ok() -> Self {
        Self {
            body: OK,
            ..Self::status(200)
        }
        .header("content-type", "application/json")
    }
    /// A redirect status with one `Location`.
    fn to(status: u16, location: &str) -> Self {
        Self::status(status).header("location", location)
    }
    fn header(mut self, name: &str, value: impl AsRef<[u8]>) -> Self {
        self.headers.push(Header::new(name, value));
        self
    }
    fn result(self) -> TransportResult {
        let mut response = ResponseInput::new(
            self.status,
            self.headers,
            Arc::from(self.body.as_bytes()),
            BodyState::Complete,
            Provenance::Wire,
        );
        response.header_delivery = self.delivery;
        response.opaque_redirect = self.opaque;
        TransportResult {
            upload: UploadState::Complete,
            response: Some(response),
            error_detail: None,
        }
    }
}

/// What reached the host: every `Transport::execute` entry, and every request that
/// reached the HTTP callback after the client's admission.
#[derive(Default)]
struct Log {
    executed: RefCell<Vec<Hop>>,
    dispatched: RefCell<Vec<Hop>>,
}
impl Log {
    fn executed(&self) -> usize {
        self.executed.borrow().len()
    }
    fn dispatched(&self) -> Vec<Hop> {
        self.dispatched.borrow().clone()
    }
}

type Host = Box<dyn Fn(PreparedRequest, Cancellation) -> Pin<Box<dyn Future<Output = Outcome>>>>;

/// A host that answers each request with the next reply, in order.
fn host(log: &Rc<Log>, replies: Vec<Reply>) -> Host {
    let log = log.clone();
    let replies = Rc::new(RefCell::new(VecDeque::from(replies)));
    Box::new(move |request, cancellation| {
        log.executed.borrow_mut().push(Hop::of(&request));
        let reply = replies
            .borrow_mut()
            .pop_front()
            .expect("a reply for each hop");
        let log = log.clone();
        Box::pin(async move {
            request
                .invoke(
                    cancellation,
                    &HostCapabilities::programmable(),
                    move |wire, _| async move {
                        log.dispatched.borrow_mut().push(Hop::of(&wire));
                        reply.result()
                    },
                )
                .await
        })
    })
}

/// Invoke one binding with one input value and return its output text, if any.
async fn call<P: binding::BindingInvoker>(
    provider: &P,
    source: &Source,
    path: &str,
    method: &str,
    input: &str,
    context: Option<&Value>,
    control: &CancellationToken,
) -> Result<Option<String>, InvocationError> {
    let binding = BindingValue {
        content: Some(
            JsonValue::parse(json!({"path": path, "method": method}).to_string()).unwrap(),
        ),
        ..Default::default()
    };
    let context = context.map(|c| JsonValue::parse(c.to_string()).unwrap());
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke_with_control(
        BindingRequest {
            source,
            binding: &binding,
            context: context.as_ref(),
        },
        control,
    );
    let _ = job.write(JsonValue::parse(input).unwrap()).await;
    job.close();
    let mut output = None;
    while let Some(value) = job.next().await {
        let value = value?;
        assert!(output.is_none(), "at most one value");
        output = Some(value.text().to_owned());
    }
    job.outcome().expect("terminal")?;
    Ok(output)
}

/// Run one invocation against scripted replies, returning its result and the log.
async fn run(
    source: &Source,
    path: &str,
    method: &str,
    input: &str,
    context: Option<&Value>,
    replies: Vec<Reply>,
) -> (Result<Option<String>, InvocationError>, Rc<Log>) {
    run_with(
        Options::default(),
        source,
        path,
        method,
        input,
        context,
        replies,
    )
    .await
}
async fn run_with(
    options: Options,
    source: &Source,
    path: &str,
    method: &str,
    input: &str,
    context: Option<&Value>,
    replies: Vec<Reply>,
) -> (Result<Option<String>, InvocationError>, Rc<Log>) {
    let log = Rc::new(Log::default());
    let provider = OpenApi::new(options)
        .unwrap()
        .with_transport(host(&log, replies));
    let control = CancellationToken::new();
    let result = call(&provider, source, path, method, input, context, &control).await;
    (result, log)
}

fn failed(result: Result<Option<String>, InvocationError>) {
    let error = result.unwrap_err();
    assert_eq!(error.code(), FAILED);
    assert!(error.data().is_none());
}

#[tokio::test]
async fn each_redirect_status_applies_its_method_and_content_rule() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        for status in [301, 302, 303, 307, 308] {
            for method in ["get", "post", "put", "patch", "delete"] {
                let content = matches!(method, "post" | "put" | "patch");
                let input = if content { r#"{"body":{"n":1}}"# } else { "{}" };
                let (result, log) = run(
                    &src,
                    "/items",
                    method,
                    input,
                    None,
                    vec![Reply::to(status, "/moved"), Reply::ok()],
                )
                .await;
                let case = format!("{version} {status} {method}");
                assert_eq!(result.unwrap().as_deref(), Some(OK), "{case}");
                let hops = log.dispatched();
                assert_eq!(hops.len(), 2, "{case}");
                assert_eq!(log.executed(), 2, "{case}");
                let (first, next) = (&hops[0], &hops[1]);
                assert_eq!(first.target, format!("{ORIGIN}/items"), "{case}");
                assert_eq!(next.target, format!("{ORIGIN}/moved"), "{case}");
                // 303 makes every method GET without content; 301 and 302 do so for POST.
                let to_get = status == 303 || (matches!(status, 301 | 302) && method == "post");
                let method = method.to_ascii_uppercase();
                if to_get {
                    assert_eq!(next.method, "GET", "{case}");
                    assert_eq!(next.body, None, "{case}");
                    assert_eq!(next.header("content-type"), None, "{case}");
                } else {
                    assert_eq!(next.method, method, "{case}");
                    assert_eq!(next.body, first.body, "{case}");
                    assert_eq!(
                        next.header("content-type"),
                        first.header("content-type"),
                        "{case}"
                    );
                }
                if content {
                    assert_eq!(first.body.as_deref(), Some(r#"{"n":1}"#), "{case}");
                    assert_eq!(first.header("content-type"), Some("application/json"));
                }
            }
        }
    }
}

#[tokio::test]
async fn a_chain_of_hops_follows_each_location_in_turn() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        let (result, log) = run(
            &src,
            "/items",
            "post",
            r#"{"body":{"n":1}}"#,
            None,
            vec![
                Reply::to(307, "https://api.example.test/a"),
                // Relative to the previous target.
                Reply::to(308, "b?q=1"),
                Reply::to(301, "https://other.example.test/c"),
                // The fragment is not part of the next target.
                Reply::to(303, "/d#part"),
                Reply {
                    body: "done",
                    ..Reply::status(200)
                }
                .header("content-type", "text/plain"),
            ],
        )
        .await;
        // The final response is read under the response rules.
        assert_eq!(result.unwrap().as_deref(), Some(r#""done""#), "{version}");
        let hops: Vec<_> = log
            .dispatched()
            .into_iter()
            .map(|h| (h.method, h.target, h.body))
            .collect();
        let body = Some(r#"{"n":1}"#.to_owned());
        assert_eq!(
            hops,
            [
                ("POST".into(), format!("{ORIGIN}/items"), body.clone()),
                ("POST".into(), format!("{ORIGIN}/a"), body.clone()),
                ("POST".into(), format!("{ORIGIN}/b?q=1"), body),
                ("GET".into(), "https://other.example.test/c".into(), None),
                ("GET".into(), "https://other.example.test/d".into(), None),
            ],
            "{version}"
        );
        assert_eq!(log.executed(), 5);
        let (result, log) = run(
            &src,
            "/items",
            "get",
            "{}",
            None,
            vec![Reply::to(302, "/a"), Reply::to(307, "/b"), Reply::ok()],
        )
        .await;
        assert_eq!(result.unwrap().as_deref(), Some(OK));
        assert_eq!(log.executed(), 3);
        assert_eq!(log.dispatched().len(), 3);
    }
}

#[tokio::test]
async fn twenty_redirects_are_followed_and_a_twenty_first_fails() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        let chain = |redirects: usize| {
            let mut replies: Vec<_> = (0..redirects)
                .map(|n| Reply::to(302, &format!("/hop/{n}")))
                .collect();
            replies.push(Reply::ok());
            replies
        };
        let (result, log) = run(&src, "/items", "get", "{}", None, chain(20)).await;
        assert_eq!(result.unwrap().as_deref(), Some(OK), "{version}");
        assert_eq!(log.executed(), 21);
        assert_eq!(log.dispatched().len(), 21);
        let (result, log) = run(&src, "/items", "get", "{}", None, chain(21)).await;
        failed(result);
        assert_eq!(
            log.executed(),
            21,
            "the twenty-first redirect is not followed"
        );
        // The limit is the client's `Limits::redirects`, taken from the adapter's options.
        for (limit, redirects, ok) in [(2, 2, true), (2, 3, false), (0, 1, false)] {
            let mut options = Options::default();
            options.limits.redirects = limit;
            let (result, log) =
                run_with(options, &src, "/items", "get", "{}", None, chain(redirects)).await;
            assert_eq!(result.is_ok(), ok, "{limit} {redirects}");
            assert_eq!(log.executed(), limit.min(redirects) + 1);
        }
    }
}

/// Credentials for one scheme, granted to the original origin only.
fn credentials(point: &str, scheme: &str) -> Value {
    json!({
        "credentials": {scheme: "fixture-credential"},
        "configuration": {point: {"origins": {scheme: [ORIGIN]}}}
    })
}

#[tokio::test]
async fn credentials_stay_on_the_same_origin_and_never_reach_another() {
    for (kind, version, point) in KINDS {
        let src = source(kind, version);
        for (path, scheme, header, value) in [
            (
                "/token",
                "Token",
                "authorization",
                "Bearer fixture-credential",
            ),
            ("/key", "Key", "x-key", "fixture-credential"),
        ] {
            let context = credentials(point, scheme);
            let case = format!("{version} {scheme}");
            // The same origin keeps the credential, wherever the path goes.
            let (result, log) = run(
                &src,
                path,
                "get",
                "{}",
                Some(&context),
                vec![
                    Reply::to(302, "/elsewhere"),
                    Reply::to(307, "https://API.example.test:443/again"),
                    Reply::ok(),
                ],
            )
            .await;
            assert_eq!(result.unwrap().as_deref(), Some(OK), "{case}");
            let hops = log.dispatched();
            assert_eq!(hops.len(), 3, "{case}");
            for hop in &hops {
                assert_eq!(hop.header(header), Some(value), "{case} {}", hop.target);
            }
            // Another host or port is another origin: the credential is removed there, and
            // it does not return when a later hop comes back to the original origin.
            for elsewhere in [
                "https://other.example.test/elsewhere",
                "https://api.example.test:8443/elsewhere",
            ] {
                let (result, log) = run(
                    &src,
                    path,
                    "get",
                    "{}",
                    Some(&context),
                    vec![
                        Reply::to(302, elsewhere),
                        Reply::to(302, "https://api.example.test/back"),
                        Reply::ok(),
                    ],
                )
                .await;
                assert_eq!(result.unwrap().as_deref(), Some(OK), "{case} {elsewhere}");
                let hops = log.dispatched();
                assert_eq!(hops.len(), 3, "{case} {elsewhere}");
                assert_eq!(hops[0].header(header), Some(value), "{case}");
                assert_eq!(hops[1].target, elsewhere, "{case}");
                for hop in &hops[1..] {
                    assert_eq!(hop.header(header), None, "{case} {}", hop.target);
                    assert!(
                        hop.headers.iter().all(|(n, _)| n != "authorization"),
                        "{case} {}",
                        hop.target
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn an_https_to_http_redirect_is_the_final_response() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        for status in [301, 302, 303, 307, 308] {
            for location in ["http://api.example.test/items", "http://127.0.0.1/private"] {
                let (result, log) = run(
                    &src,
                    "/items",
                    "get",
                    "{}",
                    None,
                    vec![Reply::to(status, location)],
                )
                .await;
                // The 3xx is read as the final response: a non-2xx, which no adaptation
                // maps, completes unsuccessfully with no data.
                failed(result);
                assert_eq!(log.executed(), 1, "{version} {status} {location}");
            }
        }
    }
}

#[tokio::test]
async fn an_unusable_location_completes_unsuccessfully() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        for reply in [
            Reply::to(302, "ftp://api.example.test/items"),
            Reply::to(302, "mailto:someone@example.test"),
            Reply::to(302, "https://user@api.example.test/items"),
            Reply::to(302, "https://[::1/items"),
            Reply::to(302, "https://api.example.test/a b"),
            Reply::to(307, "/a").header("location", "/b"),
            Reply::status(308).header("location", b"/caf\xE9"),
        ] {
            let (result, log) = run(&src, "/items", "get", "{}", None, vec![reply]).await;
            failed(result);
            assert_eq!(log.executed(), 1, "{version}");
        }
    }
}

#[tokio::test]
async fn no_location_and_statuses_that_are_not_redirects_to_follow_are_final() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        let mut replies: Vec<_> = [301, 302, 303, 307, 308]
            .into_iter()
            .map(Reply::status)
            .collect();
        replies.extend(
            [300, 304, 305, 306]
                .into_iter()
                .map(|s| Reply::to(s, "/moved")),
        );
        for reply in replies {
            let status = reply.status;
            let (result, log) = run(&src, "/items", "get", "{}", None, vec![reply]).await;
            failed(result);
            assert_eq!(log.executed(), 1, "{version} {status}");
        }
    }
}

#[tokio::test]
async fn incomplete_header_evidence_completes_unsuccessfully() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        for delivery in [HeaderDelivery::Unspecified, HeaderDelivery::Incomplete] {
            let mut reply = Reply::to(302, "/moved");
            reply.delivery = delivery;
            let (result, log) = run(&src, "/items", "get", "{}", None, vec![reply]).await;
            failed(result);
            assert_eq!(log.executed(), 1, "{version} {delivery:?}");
        }
        // Header lines the client omits beyond its bounds could hide a `Location`.
        let mut options = Options::default();
        options.limits.headers = 1;
        let reply = Reply::to(302, "/moved").header("x-extra", "1");
        let (result, log) = run_with(options, &src, "/items", "get", "{}", None, vec![reply]).await;
        failed(result);
        assert_eq!(log.executed(), 1, "{version}");
    }
}

#[tokio::test]
async fn an_opaque_redirect_completes_unsuccessfully() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        // A browser's manual redirect exposes neither its status nor its `Location`.
        let mut reply = Reply::status(0);
        reply.delivery = HeaderDelivery::Unspecified;
        reply.opaque = true;
        let (result, log) = run(&src, "/items", "get", "{}", None, vec![reply]).await;
        failed(result);
        assert_eq!(log.executed(), 1, "{version}");
        assert!(log.dispatched().len() == 1);
    }
}

#[tokio::test]
async fn each_hop_enters_the_transport_exactly_once() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        let (result, log) = run(
            &src,
            "/items",
            "put",
            r#"{"body":[1,2]}"#,
            None,
            vec![Reply::to(308, "/a"), Reply::to(302, "/b"), Reply::ok()],
        )
        .await;
        assert_eq!(result.unwrap().as_deref(), Some(OK));
        // What `execute` received is exactly what reached the HTTP callback, hop by hop.
        assert_eq!(*log.executed.borrow(), log.dispatched(), "{version}");
        assert_eq!(log.executed(), 3);
    }
}

#[tokio::test]
async fn destination_policy_runs_at_every_hop() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        let log = Rc::new(Log::default());
        let inner = host(
            &log,
            vec![
                Reply::to(302, "https://internal.example.test/admin"),
                Reply::ok(),
            ],
        );
        let refused = Rc::new(Cell::new(0));
        let refusals = refused.clone();
        // A host that admits only its allowed origin, at every request it is given.
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancellation: Cancellation| {
                if !request.target().starts_with("https://api.example.test/") {
                    refusals.set(refusals.get() + 1);
                    let refusal = Outcome::not_dispatched(
                        Diagnostic::new(Code::InvalidDestination),
                        false,
                        None,
                    );
                    return Box::pin(async move { refusal })
                        as Pin<Box<dyn Future<Output = Outcome>>>;
                }
                inner(request, cancellation)
            },
        );
        let control = CancellationToken::new();
        failed(call(&provider, &src, "/items", "get", "{}", None, &control).await);
        assert_eq!(refused.get(), 1, "{version}");
        assert_eq!(log.executed(), 1);
        assert_eq!(log.dispatched().len(), 1);
    }
}

#[tokio::test]
async fn cancellation_between_hops_enters_no_further_hop() {
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        let log = Rc::new(Log::default());
        let inner = host(&log, vec![Reply::to(302, "/moved"), Reply::ok()]);
        let control = CancellationToken::new();
        let cancel = control.clone();
        // The caller cancels while the first hop's redirect is arriving.
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancellation: Cancellation| {
                let hop = inner(request, cancellation);
                let cancel = cancel.clone();
                async move {
                    let outcome = hop.await;
                    cancel.cancel();
                    outcome
                }
            },
        );
        let error = call(&provider, &src, "/items", "get", "{}", None, &control)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ERR_CANCELLED", "{version}");
        assert_eq!(log.executed(), 1);
    }
}

#[tokio::test]
async fn dropping_the_invocation_cancels_a_pending_hop() {
    struct Dropped(Rc<Cell<bool>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    for (kind, version, _) in KINDS {
        let src = source(kind, version);
        let log = Rc::new(Log::default());
        let first = host(&log, vec![Reply::to(307, "/moved")]);
        let hops = Rc::new(Cell::new(0));
        let dropped = Rc::new(Cell::new(false));
        let token = Rc::new(RefCell::new(None::<Cancellation>));
        let (entered, guard, seen) = (hops.clone(), dropped.clone(), token.clone());
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancellation: Cancellation| {
                entered.set(entered.get() + 1);
                if entered.get() == 1 {
                    return first(request, cancellation);
                }
                // The second hop never completes on its own.
                *seen.borrow_mut() = Some(cancellation);
                let guard = Dropped(guard.clone());
                Box::pin(async move {
                    let _guard = guard;
                    std::future::pending::<Outcome>().await
                }) as Pin<Box<dyn Future<Output = Outcome>>>
            },
        );
        let binding = BindingValue {
            content: Some(JsonValue::parse(r#"{"path":"/items","method":"get"}"#).unwrap()),
            ..Default::default()
        };
        let invoker = binding::Invoker::new(&provider);
        let mut job = invoker.invoke(BindingRequest {
            source: &src,
            binding: &binding,
            context: None,
        });
        job.write(JsonValue::parse("{}").unwrap()).await.unwrap();
        job.close();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), job.next())
                .await
                .is_err(),
            "{version}"
        );
        assert_eq!(hops.get(), 2);
        assert!(!dropped.get());
        drop(job);
        assert!(dropped.get(), "{version}: the pending hop is dropped");
        assert!(token.borrow().as_ref().unwrap().is_cancelled());
    }
}
