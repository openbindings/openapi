//! Qualification executable. Compare additional adapter work with the direct client.
use dynamic_openapi_client::{Document, Limits};
use openbindings::{JsonValue, Source};
use openbindings_openapi::{EXPERIMENTAL_KIND, OpenApi};
use openbindings_source_inspection::Inspector;
use serde_json::{Value, json};
use std::{
    future::Future,
    hint::black_box,
    pin::pin,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

fn ready<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!(
            "qualification expected synchronous inspection; revise harness if implementation suspends"
        ),
    }
}
fn inventory(document: &Document) -> Vec<(String, String, Option<String>)> {
    document
        .operations()
        .map(|o| (o.path().to_owned(), o.method().as_str().to_owned(), o.id()))
        .collect()
}
fn sample<T>(iterations: usize, mut f: impl FnMut() -> T) -> (f64, usize) {
    let start = Instant::now();
    let mut completed = 0;
    loop {
        for _ in 0..iterations {
            drop(black_box(f()));
        }
        completed += iterations;
        if start.elapsed() >= Duration::from_millis(50) {
            break;
        }
    }
    (
        start.elapsed().as_nanos() as f64 / completed as f64,
        completed,
    )
}
fn owners() -> (usize, usize) {
    (
        dynamic_openapi_client::live_json_owners(),
        openbindings_internal_json::backend::live_arenas(),
    )
}
fn main() {
    let mode = std::env::args().nth(1).expect("baseline or adapter");
    assert!(matches!(mode.as_str(), "baseline" | "adapter"));
    let mut workloads = vec![];
    for (count, iterations) in [(1usize, 1000usize), (32, 100), (512, 10)] {
        let paths:serde_json::Map<String,Value>=(0..count).map(|i|(format!("/item/{i}"),json!({"get":{"operationId":format!("item{i}"),"responses":{"200":{"description":"OK"}}}}))).collect();
        let bytes=serde_json::to_vec(&json!({"openapi":"3.1.2","info":{"title":"Generated baseline","version":"1"},"paths":paths})).unwrap();
        let document = Document::parse(&bytes, None, Limits::default()).unwrap();
        let carrier =
            serde_json::to_string(&json!({"document": std::str::from_utf8(&bytes).unwrap()}))
                .unwrap();
        let source = Source {
            kind: EXPERIMENTAL_KIND.into(),
            content: Some(JsonValue::parse(&carrier).unwrap()),
            ..Default::default()
        };
        let inspector = Inspector::new(OpenApi::default());
        let check = |inventory: openbindings_source_inspection::SourceInspection| {
            assert!(inventory.exhaustive);
            assert_eq!(inventory.targets.len(), count);
            inventory
        };
        let baseline_cold = || {
            let d = Document::parse(black_box(&bytes), None, Limits::default()).unwrap();
            let values = inventory(&d);
            assert_eq!(values.len(), count);
            values
        };
        let adapter_cold = || {
            check(ready(Inspector::new(OpenApi::default()).inspect(black_box(&source))).unwrap())
        };
        for _ in 0..5 {
            if mode == "baseline" {
                black_box(baseline_cold());
                black_box(inventory(&document));
            } else {
                black_box(adapter_cold());
                black_box(check(ready(inspector.inspect(&source)).unwrap()));
            }
        }
        let run = |cold: bool| {
            if mode == "baseline" {
                if cold {
                    sample(iterations, baseline_cold)
                } else {
                    sample(iterations * 5, || inventory(black_box(&document)))
                }
            } else if cold {
                sample(iterations, adapter_cold)
            } else {
                sample(iterations * 5, || {
                    check(ready(inspector.inspect(black_box(&source))).unwrap())
                })
            }
        };
        let mut samples = vec![];
        for index in 0..7 {
            let (cold, warm) = if index % 2 == 0 {
                (run(true), run(false))
            } else {
                let warm = run(false);
                (run(true), warm)
            };
            samples.push(json!({"coldNs":cold.0,"warmNs":warm.0,"coldIterations":cold.1,"warmIterations":warm.1}));
        }
        let before = owners();
        for _ in 0..1000 {
            if mode == "baseline" {
                drop(baseline_cold());
            } else {
                drop(adapter_cold());
            }
        }
        let after = owners();
        assert_eq!(before, after);
        workloads.push(json!({"operations":count,"sourceBytes":bytes.len(),"samples":samples,"ownerCycles":1000,"clientOwnerDelta":after.0 as i64-before.0 as i64,"sdkOwnerDelta":after.1 as i64-before.1 as i64,"clientAccountedRetainedBytes":document.retained_bytes(),"resultJsonBytes":check(ready(inspector.inspect(&source)).unwrap()).to_json().unwrap().text().len()}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"mode":mode,"workloads":workloads,"scope":"native release, generation/input-carrier construction excluded, result destruction included; logical owner accounting is not total heap or RSS"})).unwrap());
}
