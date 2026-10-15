//! Source scaling and negative-path CPU diagnostics; no equivalent direct denominator.
use openapi_json_path_consumer::{auth, source};
use openbindings::JsonValue;
use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
use openbindings_openapi::OpenApi;
use openbindings_source_inspection::Inspector;
use std::{
    collections::BTreeMap,
    future::Future,
    hint::black_box,
    rc::Rc,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
fn ready<F: Future>(future: F) -> F::Output {
    let mut f = std::pin::pin!(future);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("diagnostic host unexpectedly pending"),
    }
}
fn samples(mut f: impl FnMut()) -> Vec<f64> {
    f();
    (0..7)
        .map(|_| {
            let start = Instant::now();
            let mut count = 0;
            while start.elapsed() < Duration::from_millis(50) {
                f();
                count += 1;
            }
            start.elapsed().as_nanos() as f64 / count as f64
        })
        .collect()
}
fn main() {
    let mut rows = vec![];
    for version in ["3.0.4", "3.1.2"] {
        let (kind, fixture, _, _) = auth::edition(version == "3.0.4");
        for n in [1, 32, 512] {
            let paths: BTreeMap<_, _> = (0..n)
                .map(|i| (format!("/p{i}"), serde_json::json!({"get":{"responses":{"200":{"description":"JSON","content":{"application/json":{}}}}}})))
                .collect();
            let text = serde_json::json!({"openapi":version,"info":{"title":"Auth source scaling","version":"1"},"servers":[{"url":"https://measure.invalid"}],"security":[{"Token":[],"Key":[]}],"components":{"securitySchemes":{"Token":{"type":"http","scheme":"bearer"},"Key":{"type":"apiKey","in":"header","name":"X-Key"}}},"paths":paths}).to_string();
            let src = source(kind, &text);
            let p = OpenApi::default();
            let sources = [SynthesisSource {
                name: Some("api"),
                source: &src,
            }];
            let req = SynthesisRequest {
                sources: &sources,
                ..Default::default()
            };
            let initial = ready(Synthesizer::new(&p).synthesize_with_coverage(&req)).unwrap();
            assert_eq!(
                initial
                    .document()
                    .as_document()
                    .value()
                    .get("operations")
                    .unwrap()
                    .members()
                    .unwrap()
                    .count(),
                n
            );
            for cold in [false, true] {
                for stage in ["inspection", "synthesis-with-coverage"] {
                    let timing = samples(|| {
                        let fresh;
                        let provider = if cold {
                            fresh = OpenApi::default();
                            &fresh
                        } else {
                            &p
                        };
                        if stage == "inspection" {
                            black_box(ready(Inspector::new(provider).inspect(&src)).unwrap());
                        } else {
                            black_box(
                                ready(Synthesizer::new(provider).synthesize_with_coverage(&req))
                                    .unwrap(),
                            );
                        }
                    });
                    rows.push(serde_json::json!({"version":version,"stage":stage,"operations":n,"cold":cold,"source_bytes":src.content.as_ref().unwrap().bytes().len(),"samples_ns":timing,"status":"diagnostic"}));
                }
            }
        }
        let src = auth::source(kind, fixture);
        for (stage, path, context, expected, host_calls) in [
            (
                "missing-credentials",
                "/items",
                None,
                Some("CONTEXT_REQUIRED"),
                0,
            ),
            (
                "wrong-origin",
                "/items",
                Some(
                    serde_json::json!({"credentials":{"ServiceToken":"fixture-bearer-A"},"configuration":{format!("{kind}.security"):{"schemes":["ServiceToken"],"origins":{"ServiceToken":["https://elsewhere.invalid"]}}}}),
                ),
                Some("ERR_REFUSED"),
                0,
            ),
            (
                "security-selection",
                "/choice",
                None,
                Some("CONTEXT_REQUIRED"),
                0,
            ),
            (
                "anonymous-selection",
                "/optional",
                Some(auth::context(kind, &[])),
                None,
                1,
            ),
        ] {
            let context = context.map(auth::value);
            let seen = Rc::new(auth::Seen::default());
            let p = OpenApi::default().with_transport(auth::host(seen.clone(), 200, auth::OUTPUT));
            let run = || {
                seen.calls.set(0);
                seen.wires.borrow_mut().clear();
                let result = ready(auth::binding_call(
                    &p,
                    &src,
                    path,
                    "get",
                    context.as_ref(),
                    &["{}"],
                ));
                assert_eq!(result.as_ref().err().map(|e| e.code()), expected);
                assert_eq!(seen.calls.get(), host_calls);
                let _ = black_box(result);
            };
            // Verify exact terminal/effect behavior before timing and warm source preparation.
            run();
            rows.push(serde_json::json!({"version":version,"stage":stage,"context_bytes":context.as_ref().map_or(0, |v: &JsonValue| v.bytes().len()),"samples_ns":samples(run),"status":"diagnostic"}));
        }
    }
    println!(
        "{}",
        serde_json::json!({"rows":rows,"notes":["Cold includes fresh provider and source preparation; warm retains public source analysis only.","No direct baseline claims equivalent synthesis coverage, so these source rows are diagnostic under the registered rule.","Negative-path rows include effect assertions and helper ownership/destruction; no network/heap/RSS claim."]})
    );
}
