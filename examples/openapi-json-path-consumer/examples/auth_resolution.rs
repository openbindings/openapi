//! Frozen measurement-resolution protocol. Manufactured credentials only.
//! Product work mirrors auth_measure.rs; historical harness remains untouched.
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
    fn output(&self, value: &JsonValue, expected: &str) {
        if self.active.get() {
            assert_eq!(value.text(), expected);
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
fn factory(
    row: &Row,
    authenticated: bool,
    direct: bool,
    fixture: &Fixture,
) -> (Box<dyn FnMut()>, usize, String, String) {
    let Row {
        version,
        method,
        token_bytes,
        mechanism,
        layer,
        ..
    } = *row;
    let kind = if version == "3.0.4" {
        openbindings_openapi::EXPERIMENTAL_KIND_30
    } else {
        openbindings_openapi::EXPERIMENTAL_KIND
    };
    let mut functions: Vec<(&str, Box<dyn FnMut()>)> = vec![];
    let mut context_bytes = 0;
    let text = description(
        version,
        method,
        if authenticated { mechanism } else { "none" },
        1,
    );
    let src = source(kind, &text);
    let doc = client::Document::parse(text.as_bytes(), None, client::Limits::default()).unwrap();
    let op = doc.operations().next().unwrap();
    let input = fixture.input.clone();
    let body = fixture.body.clone();
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
            credentials.insert(name.into(), serde_json::json!("a".repeat(token_bytes)));
            origins.insert(name.into(), serde_json::json!(["https://measure.invalid"]));
        }
    }
    let context=JsonValue::parse(serde_json::json!({"credentials":credentials,"configuration":{format!("{kind}.security"):{"schemes":names,"origins":origins}}}).to_string()).unwrap();
    if authenticated {
        context_bytes = context.bytes().len();
    }
    let context_text = context.text().to_owned();
    let source_text = text.clone();
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
    if direct {
        functions.push((
            "call",
            Box::new(move || {
                let mut supplied = op.request();
                supplied = supplied.server(0);
                if let Some(body) = input.get("body") {
                    supplied = supplied.json_exact(
                        &client::exact::Value::parse(body.text(), client::Limits::default())
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
                let value = JsonValue::parse(exact.source()).unwrap();
                oracle.output(&value, &body);
                black_box(value);
                oracle.finish();
            }),
        ));
    } else {
        let expected_body = body.clone();
        let host_oracle = oracle.clone();
        let p = OpenApi::default().with_transport(
            move |request: client::transport::PreparedRequest, cancel: client::Cancellation| {
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
                    JsonValue::parse(serde_json::json!({"path":"/p0","method":method}).to_string())
                        .unwrap(),
                ),
                ..Default::default()
            };
            functions.push((
                "call",
                Box::new(move || {
                    let mut call = facade.invoke(BindingRequest {
                        source: &src,
                        binding: &binding,
                        context: Some(&context),
                    });
                    ready(call.write(input.clone())).unwrap();
                    call.close();
                    let value = ready(call.next()).unwrap().unwrap();
                    oracle.output(&value, &expected_body);
                    black_box(value);
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
                            serde_json::json!({"path":"/p0","method":method}).to_string(),
                        )
                        .unwrap(),
                    ),
                    ..Default::default()
                },
            )]));
            let facade =
                operation::Invoker::new(ParsedDocument::from_json(draft.to_json().unwrap()), p);
            functions.push((
                "call",
                Box::new(move || {
                    let mut call =
                        facade.invoke(operation::Request::operation("call").with_context(&context));
                    ready(call.write(input.clone())).unwrap();
                    call.close();
                    let value = ready(call.next()).unwrap().unwrap();
                    oracle.output(&value, &expected_body);
                    black_box(value);
                    assert!(ready(call.next()).is_none());
                    oracle.finish();
                }),
            ));
        }
    }
    (
        functions.pop().unwrap().1,
        context_bytes,
        source_text,
        context_text,
    )
}

// Measurement executable only: cooperative signal handling preserves partial
// blocks at a complete-call boundary. No signal hooks enter the SDK.
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn request_stop(_: i32) {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}
unsafe extern "C" {
    fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
}
fn stopping() -> bool {
    STOP.load(std::sync::atomic::Ordering::Relaxed)
}
const SAMPLER: &str = "resolution-fixed-cyclic@1";
const CAP: usize = 16384;
const CYCLE_CAP: usize = 146;
const SLOTS: [usize; 16] = [0, 0, 1, 0, 2, 0, 3, 1, 1, 2, 1, 3, 2, 2, 3, 3];
#[derive(Clone, Copy)]
struct Row {
    version: &'static str,
    workload: &'static str,
    method: &'static str,
    bytes: usize,
    token_bytes: usize,
    mechanism: &'static str,
    layer: &'static str,
}
fn rows(phase: &str) -> Vec<Row> {
    let mut result = vec![];
    for version in ["3.0.4", "3.1.2"] {
        for (workload, method, bytes, token_bytes, mechanism) in [
            ("bearer-get", "get", 1024, 64, "bearer"),
            ("key-post", "post", 1024, 64, "key"),
            ("and-post", "post", 65536, 64, "and"),
            ("large-token", "get", 1024, 4096, "bearer"),
            ("anonymous-get-large", "get", 65536, 0, "none"),
            ("anonymous-post-large", "post", 65536, 0, "none"),
        ] {
            if phase == "R" && workload != "and-post" {
                continue;
            }
            for layer in ["binding", "operation"] {
                result.push(Row {
                    version,
                    workload,
                    method,
                    bytes,
                    token_bytes,
                    mechanism,
                    layer,
                });
            }
        }
    }
    result
}
struct Fixture {
    input: JsonValue,
    body: std::sync::Arc<String>,
}
impl Fixture {
    fn new(row: &Row) -> Self {
        let body = format!("\"{}\"", "x".repeat(row.bytes - 2));
        let input = JsonValue::parse(if row.method == "post" {
            format!("{{\"body\":{body}}}")
        } else {
            "{}".into()
        })
        .unwrap();
        Self {
            input,
            body: std::sync::Arc::new(body),
        }
    }
}
struct Object {
    id: usize,
    fixture: Rc<Fixture>,
    f: Box<dyn FnMut()>,
    meta: serde_json::Value,
}
struct Cohort {
    objects: Vec<Object>,
    configs: Vec<Config>,
}
struct Config {
    name: &'static str,
    map: [usize; 4],
    blocks: Vec<RawBlock>,
}
#[derive(Clone)]
struct RawBlock {
    sample: usize,
    cycle: usize,
    position: usize,
    slot: usize,
    predecessor: usize,
    ns: u128,
    calls: usize,
    prefix: bool,
    ordinal: usize,
}
fn cycle_order(
    sample: usize,
    row: usize,
    process: usize,
    config: usize,
    cohort: usize,
) -> [usize; 16] {
    let mut slots = SLOTS;
    slots.rotate_left((sample + 5 * row + 3 * process + 7 * config + 11 * cohort) % 16);
    if (sample + process + cohort + config) % 2 == 1 {
        slots.reverse();
    }
    slots
}
fn configuration_order(
    sample: usize,
    row: usize,
    process: usize,
    cohort: usize,
    n: usize,
    phase: &str,
) -> Vec<usize> {
    let mut order: Vec<_> = (0..n).collect();
    order.rotate_left(if phase == "R" {
        (sample + 2 * process + row + cohort) % n
    } else {
        (sample + process + row) % n
    });
    if (sample + process + if phase == "R" { cohort } else { row }) % 2 == 1 {
        order.reverse();
    }
    order
}
fn construct(row: &Row, phase: &str, cohort: usize, anon: bool) -> Cohort {
    let extra = phase == "R" || (!anon && row.mechanism != "none");
    let n = if extra { 8 } else { 4 };
    let common = Rc::new(Fixture::new(row));
    let mut order: Vec<_> = (0..n).collect();
    if cohort == 1 {
        order.reverse();
    }
    let mut objects = vec![];
    for (rank, id) in order.into_iter().enumerate() {
        let separate = phase == "R" && id >= 4;
        let fixture = if separate {
            Rc::new(Fixture::new(row))
        } else {
            common.clone()
        };
        let direct = id % 4 < 2;
        let authenticated =
            phase == "Q" && !anon && row.mechanism != "none" && (id == 1 || id == 3);
        let (f, context_bytes, source_text, context_text) =
            factory(row, authenticated, direct, &fixture);
        let pointer = (&*f as *const dyn FnMut() as *const ()) as usize;
        let meta = serde_json::json!({"id":id,"constructor_rank":rank,"fixture_id":if separate {id} else {0},"input_pointer":fixture.input.bytes().as_ptr() as usize,"response_pointer":fixture.body.as_ptr() as usize,"object_pointer":pointer,"direct":direct,"authenticated":authenticated,"context_bytes":context_bytes,"source":source_text,"context":context_text});
        objects.push(Object {
            id,
            fixture,
            f,
            meta,
        });
    }
    objects.sort_by_key(|o| o.id);
    // Keep all fixture and closure allocations alive while checking identities.
    for a in 0..n {
        for b in 0..n {
            let shared = phase != "R" || (a < 4 && b < 4) || a == b;
            assert_eq!(Rc::ptr_eq(&objects[a].fixture, &objects[b].fixture), shared);
            assert_eq!(
                objects[a].fixture.input.bytes().as_ptr()
                    == objects[b].fixture.input.bytes().as_ptr(),
                shared
            );
            assert_eq!(
                std::sync::Arc::ptr_eq(&objects[a].fixture.body, &objects[b].fixture.body),
                shared
            );
            if a != b {
                assert_ne!(
                    objects[a].meta["object_pointer"],
                    objects[b].meta["object_pointer"]
                );
            }
        }
    }
    let specs: Vec<_> = if phase == "R" {
        vec![
            ("R-alias", [0, 0, 2, 2]),
            ("R-common-normal", [0, 1, 2, 3]),
            ("R-common-swapped", [1, 0, 3, 2]),
            ("R-separate-normal", [4, 5, 6, 7]),
        ]
    } else if extra {
        vec![
            ("Q-real", [0, 1, 2, 3]),
            ("Q-null-normal", [4, 5, 6, 7]),
            ("Q-null-swapped", [5, 4, 7, 6]),
        ]
    } else {
        vec![("Q-real", [0, 1, 2, 3])]
    };
    let configs = specs
        .into_iter()
        .map(|(name, map)| Config {
            name,
            map,
            blocks: Vec::with_capacity(CAP),
        })
        .collect();
    Cohort { objects, configs }
}
fn sample(
    cohort: &mut Cohort,
    ci: usize,
    si: usize,
    ri: usize,
    process: usize,
    hi: usize,
    k: usize,
    target: u128,
    deadline: Instant,
    ordinal: &mut usize,
) -> Result<(), &'static str> {
    if stopping() {
        return Err("interrupted");
    }
    if Instant::now() >= deadline {
        return Err("wall-time-cap");
    }
    let slots = cycle_order(si, ri, process, ci, hi);
    let config = &mut cohort.configs[ci];
    if config.blocks.len() + 1 + 16 * CYCLE_CAP > CAP {
        return Err("record-cap");
    }
    let slot = slots[15];
    let f = &mut cohort.objects[config.map[slot]].f;
    for _ in 0..k {
        f();
    }
    config.blocks.push(RawBlock {
        sample: si,
        cycle: 0,
        position: 0,
        slot,
        predecessor: slot,
        ns: 0,
        calls: k,
        prefix: true,
        ordinal: *ordinal,
    });
    *ordinal += 1;
    let mut elapsed = [0_u128; 4];
    for cycle in 0..CYCLE_CAP {
        for (position, &slot) in slots.iter().enumerate() {
            if stopping() {
                return Err("interrupted");
            }
            if Instant::now() >= deadline {
                return Err("wall-time-cap");
            }
            // Resolve physical object once before the clock. Same-object labels
            // really borrow the same retained closure; every path has this dispatch.
            let f = &mut cohort.objects[config.map[slot]].f;
            let start = Instant::now();
            for _ in 0..k {
                f();
            }
            let ns = start.elapsed().as_nanos();
            elapsed[slot] += ns;
            config.blocks.push(RawBlock {
                sample: si,
                cycle,
                position,
                slot,
                predecessor: slots[(position + 15) % 16],
                ns,
                calls: k,
                prefix: false,
                ordinal: *ordinal,
            });
            *ordinal += 1;
        }
        if elapsed.iter().all(|n| *n >= target) {
            return Ok(());
        }
    }
    Err("cycle-cap")
}
fn serialized(cohort: &Cohort, hi: usize) -> serde_json::Value {
    let configs: Vec<_>=cohort.configs.iter().enumerate().map(|(ci,c)|{
        let mut samples=vec![vec![];4];
        for si in 0..7 {for (slot,s) in samples.iter_mut().enumerate() {
            let (ns,n)=c.blocks.iter().filter(|b|!b.prefix && b.sample==si && b.slot==slot).fold((0_u128,0_usize),|(ns,n),b|(ns+b.ns,n+b.calls));
            if n>0 {s.push(ns as f64/n as f64);}
        }}
        let blocks:Vec<_>=c.blocks.iter().map(|b|serde_json::json!({"sample":b.sample,"cycle":b.cycle,"position":b.position,"slot":b.slot,"predecessor":b.predecessor,"elapsed_ns":b.ns,"calls":b.calls,"prefix":b.prefix,"ordinal":b.ordinal,"object_id":c.map[b.slot],"fixture_id":cohort.objects[c.map[b.slot]].meta["fixture_id"]})).collect();
        serde_json::json!({"id":c.name,"configuration":ci,"map":c.map,"samples_ns":samples,"blocks":blocks,"capacity":c.blocks.capacity()})
    }).collect();
    serde_json::json!({"cohort":hi,"objects":cohort.objects.iter().map(|o|&o.meta).collect::<Vec<_>>(),"configurations":configs})
}
fn main() {
    use std::io::Write;
    // POSIX SIGINT/SIGTERM on the frozen macOS target. The handler only stores
    // an atomic flag; allocation/serialization happens in ordinary control flow.
    unsafe {
        signal(2, request_stop);
        signal(15, request_stop);
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    let phase = args
        .iter()
        .find_map(|a| a.strip_prefix("phase="))
        .expect("phase required");
    assert!(phase == "R" || phase == "Q");
    let process: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("process="))
        .expect("process required")
        .parse()
        .unwrap();
    assert!(process < 3);
    let anon = args.iter().any(|a| a == "anonymous-only");
    assert!(!(phase == "R" && anon));
    let preflight = args.iter().any(|a| a == "preflight-only");
    let duration: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("remaining-seconds="))
        .expect("deadline required")
        .parse()
        .unwrap();
    assert!(duration > 0.0 && duration <= 2700.0);
    let deadline = Instant::now() + Duration::from_secs_f64(duration);
    for (ri, row) in rows(phase).iter().enumerate() {
        let mut construction = if phase == "R" {
            vec![0, 1]
        } else {
            vec![(process + ri) % 2]
        };
        if phase == "R" && (process + ri) % 2 == 1 {
            construction.reverse();
        }
        let mut cohorts: Vec<_> = construction
            .iter()
            .map(|&hi| (hi, construct(row, phase, hi, anon)))
            .collect();
        for (_, h) in &mut cohorts {
            for o in &mut h.objects {
                (o.f)();
            }
        }
        cohorts.sort_by_key(|(hi, _)| *hi);
        let mut error = None;
        let mut ordinal = 0;
        if !preflight {
            'samples: for si in 0..7 {
                let mut order: Vec<_> = (0..cohorts.len()).collect();
                if phase == "R" && (si + process + ri) % 2 == 1 {
                    order.reverse();
                }
                for i in order {
                    let (hi, h) = &mut cohorts[i];
                    for ci in configuration_order(si, ri, process, *hi, h.configs.len(), phase) {
                        if let Err(e) = sample(
                            h,
                            ci,
                            si,
                            ri,
                            process,
                            *hi,
                            if row.bytes == 65536 { 64 } else { 256 },
                            if row.workload == "and-post" {
                                800_000_000
                            } else {
                                200_000_000
                            },
                            deadline,
                            &mut ordinal,
                        ) {
                            error = Some(e);
                            break 'samples;
                        }
                    }
                }
            }
        }
        let fixture_input = cohorts[0].1.objects[0].fixture.input.text().to_owned();
        let fixture_response = cohorts[0].1.objects[0].fixture.body.to_string();
        let cohorts: Vec<_> = cohorts.iter().map(|(hi, h)| serialized(h, *hi)).collect();
        println!(
            "{}",
            serde_json::json!({"sampler":SAMPLER,"phase":phase,"process":process,"anonymous_only":anon,"preflight_only":preflight,"row":ri,"version":row.version,"workload":row.workload,"layer":row.layer,"bytes":row.bytes,"token_bytes":row.token_bytes,"construction_order":construction,"fixture_input":fixture_input,"fixture_response":fixture_response,"cohorts":cohorts,"error":error})
        );
        std::io::stdout().flush().unwrap();
        if error.is_some() {
            std::process::exit(2);
        }
        if args.iter().any(|a| a == "checkpoint-ack") {
            let mut ack = String::new();
            std::io::stdin().read_line(&mut ack).unwrap();
            assert_eq!(ack, "next\n", "checkpoint acknowledgement required");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controlled_stop_keeps_partial_records_without_calling() {
        let b = RawBlock {
            sample: 0,
            cycle: 0,
            position: 0,
            slot: 0,
            predecessor: 0,
            ns: 7,
            calls: 64,
            prefix: false,
            ordinal: 0,
        };
        let mut h = Cohort {
            objects: vec![],
            configs: vec![Config {
                name: "R-alias",
                map: [0, 0, 2, 2],
                blocks: vec![b],
            }],
        };
        let mut ordinal = 1;
        request_stop(15);
        let stopped = sample(
            &mut h,
            0,
            0,
            0,
            0,
            0,
            64,
            800_000_000,
            Instant::now() + Duration::from_secs(1),
            &mut ordinal,
        );
        STOP.store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(stopped, Err("interrupted"));
        assert_eq!(h.configs[0].blocks.len(), 1);
        assert_eq!(h.configs[0].blocks[0].ns, 7);
        assert_eq!(ordinal, 1);
        assert_eq!(
            sample(
                &mut h,
                0,
                0,
                0,
                0,
                0,
                64,
                800_000_000,
                Instant::now(),
                &mut ordinal
            ),
            Err("wall-time-cap")
        );
        assert_eq!(h.configs[0].blocks.len(), 1);
    }
    #[test]
    fn cycles_have_every_edge() {
        for s in 0..7 {
            for r in 0..24 {
                for p in 0..3 {
                    for c in 0..4 {
                        for h in 0..2 {
                            let order = cycle_order(s, r, p, c, h);
                            let mut edges = [[0; 4]; 4];
                            for i in 0..16 {
                                edges[order[(i + 15) % 16]][order[i]] += 1;
                            }
                            assert_eq!(edges, [[1; 4]; 4]);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn capacity_and_orders() {
        assert!(7 * (1 + 146 * 16) <= CAP);
        for row in 0..4 {
            for p in 0..3 {
                for h in 0..2 {
                    for s in 0..7 {
                        let mut x = configuration_order(s, row, p, h, 4, "R");
                        x.sort();
                        assert_eq!(x, [0, 1, 2, 3]);
                    }
                }
            }
        }
    }
}
