//! Registered, interleaved CPU comparisons. Manufactured credentials only.
use dynamic_openapi_client as client;
use openapi_json_path_consumer::{reply, source};
use openbindings::{Binding, Document, JsonValue, Operation, ParsedDocument};
use openbindings_invocation::{BindingRequest, BindingValue, binding, operation};
use openbindings_openapi::OpenApi;
use std::{
    cell::Cell,
    collections::BTreeMap,
    future::Future,
    hint::black_box,
    rc::Rc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
struct Oracle {
    active: Cell<bool>,
    calls: Cell<usize>,
    headers: Vec<(&'static str, String)>,
    body: Option<String>,
}
impl Oracle {
    fn record(&self, wire: &client::transport::PreparedRequest) {
        if !self.active.get() {
            return;
        }
        self.calls.set(self.calls.get() + 1);
        assert_eq!(wire.target(), "https://measure.invalid/p0");
        assert_eq!(
            wire.body().map(AsRef::as_ref),
            self.body.as_deref().map(str::as_bytes)
        );
        for name in ["authorization", "x-key"] {
            let actual: Vec<_> = wire
                .headers()
                .iter()
                .filter(|h| h.name().eq_ignore_ascii_case(name))
                .collect();
            let expected = self.headers.iter().find(|(n, _)| *n == name);
            assert_eq!(actual.len(), usize::from(expected.is_some()));
            if let Some((_, value)) = expected {
                assert_eq!(actual[0].value(), value.as_bytes());
            }
        }
    }
    fn finish(&self) {
        if self.active.replace(false) {
            assert_eq!(self.calls.get(), 1);
        }
    }
}
fn ready<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    match f.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        _ => panic!("ready host"),
    }
}
#[derive(Debug)]
struct Block {
    sample: usize,
    cycle: usize,
    order: usize,
    position: usize,
    mode: &'static str,
    elapsed_ns: u128,
    calls: u64,
}
fn timed_block(f: &mut dyn FnMut(), duration: Duration) -> (u128, u64) {
    let start = Instant::now();
    let mut n = 0;
    while start.elapsed() < duration {
        f();
        n += 1;
    }
    (start.elapsed().as_nanos(), n)
}
// Each condition occupies each position once and follows every other condition once.
const WILLIAMS: [[usize; 4]; 4] = [[0, 1, 3, 2], [1, 2, 0, 3], [2, 3, 1, 0], [3, 0, 2, 1]];
fn measure(
    functions: &mut [Measurement],
    balanced: bool,
    reverse: bool,
    standardized_preflight: bool,
) -> (BTreeMap<&'static str, Vec<f64>>, Vec<Block>) {
    let mut samples: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut blocks = vec![];
    // Historical default keeps the original per-sample warm-up. The diagnostic
    // compares both samplers with the same single separate oracle/preflight call.
    if standardized_preflight || balanced {
        for (_, f) in functions.iter_mut() {
            f();
        }
    }
    for round in 0..7 {
        if balanced {
            let mut elapsed = vec![0_u128; functions.len()];
            let mut calls = vec![0_u64; functions.len()];
            let mut cycle = 0;
            while elapsed.iter().any(|ns| *ns < 200_000_000) {
                // Always finish the whole counterbalancing cycle. Both binaries
                // use four independently constructed conditions in this mode.
                assert_eq!(functions.len(), 4);
                for (order_index, order) in WILLIAMS.iter().enumerate() {
                    for position in 0..4 {
                        let i = order[if reverse { 3 - position } else { position }];
                        let (mode, f) = &mut functions[i];
                        let (ns, n) = timed_block(&mut **f, Duration::from_millis(2));
                        elapsed[i] += ns;
                        calls[i] += n;
                        blocks.push(Block {
                            sample: round,
                            cycle,
                            order: order_index,
                            position,
                            mode,
                            elapsed_ns: ns,
                            calls: n,
                        });
                    }
                }
                cycle += 1;
            }
            for (i, (mode, _)) in functions.iter().enumerate() {
                samples
                    .entry(mode)
                    .or_default()
                    .push(elapsed[i] as f64 / calls[i] as f64);
            }
        } else {
            let mut order: Vec<_> = (0..functions.len()).collect();
            if (round % 2 == 1) ^ reverse {
                order.reverse();
            }
            for (position, i) in order.into_iter().enumerate() {
                let (mode, f) = &mut functions[i];
                if !standardized_preflight {
                    f();
                }
                let (ns, n) = timed_block(&mut **f, Duration::from_millis(50));
                samples.entry(mode).or_default().push(ns as f64 / n as f64);
                blocks.push(Block {
                    sample: round,
                    cycle: 0,
                    order: 0,
                    position,
                    mode,
                    elapsed_ns: ns,
                    calls: n,
                });
            }
        }
    }
    (samples, blocks)
}
fn description(version: &str, method: &str, auth: &str, n: usize) -> String {
    let schemes = serde_json::json!({"Token":{"type":"http","scheme":"bearer"},"Key":{"type":"apiKey","in":"header","name":"X-Key"}});
    let security = match auth {
        "bearer" => serde_json::json!([{"Token":[]}]),
        "key" => serde_json::json!([{"Key":[]}]),
        "and" => serde_json::json!([{"Token":[],"Key":[]}]),
        _ => serde_json::json!([]),
    };
    let paths:BTreeMap<_,_>=(0..n).map(|i|{let mut op=serde_json::json!({"responses":{"200":{"description":"JSON","content":{"application/json":{}}}}});if method=="post"{op["requestBody"]=serde_json::json!({"content":{"application/json":{}}});}(format!("/p{i}"),serde_json::json!({method:op}))}).collect();
    serde_json::json!({"openapi":version,"info":{"title":"Auth CPU","version":"1"},"servers":[{"url":"https://measure.invalid"}],"security":security,"components":{"securitySchemes":schemes},"paths":paths}).to_string()
}
type Measurement = (&'static str, Box<dyn FnMut()>);
fn main() {
    let anon_only = std::env::args().any(|a| a == "anonymous-only");
    let reverse = std::env::args().any(|a| a == "reverse");
    let investigation = std::env::args().any(|a| a == "investigation");
    let use_balanced = std::env::args().any(|a| a == "balanced");
    let process: usize = std::env::args()
        .find_map(|a| a.strip_prefix("process=").map(str::to_owned))
        .map(|v| v.parse().unwrap())
        .unwrap_or(0);
    let mut row_index = 0;
    let mut rows = vec![];
    for version in ["3.0.4", "3.1.2"] {
        let kind = if version == "3.0.4" {
            openbindings_openapi::EXPERIMENTAL_KIND_30
        } else {
            openbindings_openapi::EXPERIMENTAL_KIND
        };
        for (workload, method, bytes, token_bytes, mechanism) in [
            ("bearer-get", "get", 1024, 64, "bearer"),
            ("key-post", "post", 1024, 64, "key"),
            ("and-post", "post", 65536, 64, "and"),
            ("large-token", "get", 1024, 4096, "bearer"),
            ("anonymous-get-large", "get", 65536, 0, "none"),
            ("anonymous-post-large", "post", 65536, 0, "none"),
        ] {
            if investigation && workload != "and-post" {
                continue;
            }
            for layer in ["binding", "operation"] {
                let mut configurations = if investigation {
                    vec![(false, false), (false, true), (true, false), (true, true)]
                } else {
                    vec![(use_balanced, false)]
                };
                if investigation {
                    configurations.rotate_left((row_index + process) % 4);
                }
                row_index += 1;
                for (configuration_index, (balanced, null_control)) in
                    configurations.into_iter().enumerate()
                {
                    let mut functions: Vec<Measurement> = vec![];
                    let mut context_bytes = 0;
                    for mode in [
                        "direct-anonymous",
                        "direct-auth",
                        "adapter-anonymous",
                        "adapter-auth",
                    ] {
                        let authenticated = mode.ends_with("-auth")
                            && mechanism != "none"
                            && !null_control
                            && !anon_only;
                        if anon_only && !balanced && !mode.ends_with("anonymous") {
                            continue;
                        }
                        let text = description(
                            version,
                            method,
                            if authenticated { mechanism } else { "none" },
                            1,
                        );
                        let src = source(kind, &text);
                        let doc = client::Document::parse(
                            text.as_bytes(),
                            None,
                            client::Limits::default(),
                        )
                        .unwrap();
                        let op = doc.operations().next().unwrap();
                        let body = format!("\"{}\"", "x".repeat(bytes - 2));
                        let envelope = if method == "post" {
                            format!("{{\"body\":{body}}}")
                        } else {
                            "{}".into()
                        };
                        let input = JsonValue::parse(&envelope).unwrap();
                        let body = std::sync::Arc::new(body);
                        let mut credentials = serde_json::Map::new();
                        let mut origins = serde_json::Map::new();
                        let mut names = vec![];
                        if authenticated {
                            for name in match mechanism {
                                "and" => vec!["Key", "Token"],
                                "key" => vec!["Key"],
                                _ => vec!["Token"],
                            } {
                                names.push(name);
                                credentials.insert(
                                    name.into(),
                                    serde_json::json!("a".repeat(token_bytes)),
                                );
                                origins.insert(
                                    name.into(),
                                    serde_json::json!(["https://measure.invalid"]),
                                );
                            }
                        }
                        let context=JsonValue::parse(serde_json::json!({"credentials":credentials,"configuration":{format!("{kind}.security"):{"schemes":names,"origins":origins}}}).to_string()).unwrap();
                        if authenticated {
                            context_bytes = context.bytes().len();
                        }
                        let oracle = Rc::new(Oracle {
                            active: Cell::new(true),
                            calls: Cell::new(0),
                            headers: names
                                .iter()
                                .map(|name| {
                                    if *name == "Token" {
                                        (
                                            "authorization",
                                            format!("Bearer {}", "a".repeat(token_bytes)),
                                        )
                                    } else {
                                        ("x-key", "a".repeat(token_bytes))
                                    }
                                })
                                .collect(),
                            body: (method == "post").then(|| body.to_string()),
                        });
                        if mode.starts_with("direct") {
                            functions.push((
                                mode,
                                Box::new(move || {
                                    let mut supplied = op.request();
                                    supplied = supplied.server(0);
                                    if let Some(body) = input.get("body") {
                                        supplied = supplied.json_exact(
                                            &client::exact::Value::parse(
                                                body.text(),
                                                client::Limits::default(),
                                            )
                                            .unwrap(),
                                        );
                                    }
                                    if authenticated {
                                        supplied = supplied.security(0);
                                        for name in &names {
                                            let raw = context
                                                .get("credentials")
                                                .unwrap()
                                                .get(name)
                                                .unwrap()
                                                .as_str()
                                                .unwrap();
                                            supplied = supplied.credential(
                                                *name,
                                                if *name == "Token" {
                                                    client::Credential::bearer(
                                                        raw,
                                                        vec!["https://measure.invalid".into()],
                                                    )
                                                } else {
                                                    client::Credential::api_key(
                                                        raw,
                                                        vec!["https://measure.invalid".into()],
                                                    )
                                                },
                                            );
                                        }
                                    }
                                    let req = supplied.prepare().unwrap();
                                    let outcome = ready(req.invoke(
                                        client::Cancellation::default(),
                                        &client::transport::HostCapabilities::programmable(),
                                        |wire, _| {
                                            oracle.record(&wire);
                                            async { reply(&body) }
                                        },
                                    ));
                                    let exact = outcome.received_2xx_json_exact().unwrap();
                                    black_box(JsonValue::parse(exact.source()).unwrap());
                                    oracle.finish();
                                }),
                            ));
                        } else {
                            let host_oracle = oracle.clone();
                            let p = OpenApi::default().with_transport(
                            move |request: client::transport::PreparedRequest,
                                  cancel: client::Cancellation| {
                                let body = body.clone();
                                let host_oracle = host_oracle.clone();
                                async move {
                                    request
                                        .invoke(
                                            cancel,
                                            &client::transport::HostCapabilities::programmable(),
                                            |wire, _| {
                                                host_oracle.record(&wire);
                                                async { reply(&body) }
                                            },
                                        )
                                        .await
                                }
                            },
                        );
                            if layer == "binding" {
                                let facade = binding::Invoker::new(p);
                                let binding = BindingValue {
                                    content: Some(
                                        JsonValue::parse(
                                            serde_json::json!({"path":"/p0","method":method})
                                                .to_string(),
                                        )
                                        .unwrap(),
                                    ),
                                    ..Default::default()
                                };
                                functions.push((
                                    mode,
                                    Box::new(move || {
                                        let mut call = facade.invoke(BindingRequest {
                                            source: &src,
                                            binding: &binding,
                                            context: Some(&context),
                                        });
                                        ready(call.write(input.clone())).unwrap();
                                        call.close();
                                        black_box(ready(call.next()).unwrap().unwrap());
                                        assert!(ready(call.next()).is_none());
                                        oracle.finish();
                                    }),
                                ));
                            } else {
                                let mut draft = Document::new();
                                draft.sources = Some(BTreeMap::from([("api".into(), src)]));
                                draft.operations.insert("call".into(), Operation::default());
                                draft.bindings = Some(BTreeMap::from([(
                                    "call".into(),
                                    Binding {
                                        operation: "call".into(),
                                        source: "api".into(),
                                        content: Some(
                                            JsonValue::parse(
                                                serde_json::json!({"path":"/p0","method":method})
                                                    .to_string(),
                                            )
                                            .unwrap(),
                                        ),
                                        ..Default::default()
                                    },
                                )]));
                                let facade = operation::Invoker::new(
                                    ParsedDocument::from_json(draft.to_json().unwrap()),
                                    p,
                                );
                                functions.push((
                                    mode,
                                    Box::new(move || {
                                        let mut call = facade.invoke(
                                            operation::Request::operation("call")
                                                .with_context(&context),
                                        );
                                        ready(call.write(input.clone())).unwrap();
                                        call.close();
                                        black_box(ready(call.next()).unwrap().unwrap());
                                        assert!(ready(call.next()).is_none());
                                        oracle.finish();
                                    }),
                                ));
                            }
                        }
                    }
                    let (samples, blocks) =
                        measure(&mut functions, balanced, reverse, investigation);
                    let blocks: Vec<_> = blocks.into_iter().map(|b| serde_json::json!({"sample":b.sample,"cycle":b.cycle,"order":b.order,"position":b.position,"mode":b.mode,"elapsed_ns":b.elapsed_ns,"calls":b.calls})).collect();
                    rows.push(serde_json::json!({"version":version,"workload":workload,"layer":layer,"bytes":bytes,"token_bytes":token_bytes,"context_bytes":context_bytes,"samples_ns":samples,"sampler":if balanced {"balanced-200ms"} else {"legacy-50ms"},"null_control":null_control,"configuration_index":configuration_index,"blocks":blocks}));
                }
            }
        }
    }
    println!(
        "{}",
        serde_json::json!({"rows":rows,"anonymous_only":anon_only,"investigation":investigation,"process":process,"reverse":reverse})
    );
}
