//! Authenticated consumers shared by native and local Wasm qualification.
use crate::wire_json::{Value, json};
use dynamic_openapi_client::{
    Cancellation, Header, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{Document, JsonValue, Source};
use openbindings_interface_synthesis::{SynthesisRequest, SynthesisSource, Synthesizer};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding, operation};
use openbindings_openapi::{OpenApi, Options};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
pub const OUTPUT: &str = r#"{"ok":true,"n":9007199254740993,"label":"雪"}"#;
pub fn value(v: Value) -> JsonValue {
    JsonValue::parse(v.to_string()).unwrap()
}
pub fn edition(v30: bool) -> (&'static str, &'static str, &'static str, &'static str) {
    if v30 {
        (
            openbindings_openapi::EXPERIMENTAL_KIND_30,
            include_str!("auth-fixtures/service-3.0.4.json"),
            include_str!("auth-fixtures/manual-3.0.4.obi.json"),
            include_str!("auth-fixtures/variants-3.0.4.json"),
        )
    } else {
        (
            openbindings_openapi::EXPERIMENTAL_KIND,
            include_str!("auth-fixtures/service-3.1.2.json"),
            include_str!("auth-fixtures/manual-3.1.2.obi.json"),
            include_str!("auth-fixtures/variants-3.1.2.json"),
        )
    }
}
pub fn source(kind: &str, text: &str) -> Source {
    Source {
        kind: kind.into(),
        content: Some(value(
            json!({"document":text,"retrievalUri":"https://source.example.test/service.json"}),
        )),
        ..Default::default()
    }
}
pub fn context(kind: &str, names: &[&str]) -> Value {
    let mut credentials = crate::wire_json::Map::new();
    let mut origins = crate::wire_json::Map::new();
    for n in names {
        credentials.insert(
            (*n).into(),
            json!(if *n == "ServiceToken" {
                "fixture-bearer-A"
            } else {
                "fixture-key-A"
            }),
        );
        origins.insert((*n).into(), json!(["https://api.example.test"]));
    }
    json!({"credentials":credentials,"configuration":{format!("{kind}.security"):{"schemes":names,"origins":origins}}})
}
#[derive(Default)]
pub struct Seen {
    pub calls: Cell<usize>,
    pub wires: RefCell<Vec<Value>>,
}
pub fn host(
    seen: Rc<Seen>,
    status: u16,
    body: &str,
) -> impl Fn(PreparedRequest, Cancellation) -> std::pin::Pin<Box<dyn Future<Output = Outcome>>> {
    let body = body.to_owned();
    move |request, cancel| {
        let seen = seen.clone();
        let body = body.clone();
        Box::pin(async move {
            // Count adapter-to-host entry too, including a client-side dispatch refusal.
            seen.calls.set(seen.calls.get() + 1);
            request.invoke(cancel,&HostCapabilities::programmable(),move |wire,_|async move{
            seen.wires.borrow_mut().push(json!({"target":wire.target(),"headers":wire.headers().iter().map(|h|(h.name().to_lowercase(),String::from_utf8(h.value().to_vec()).unwrap())).collect::<Vec<_>>(),"body":wire.body().map(AsRef::as_ref).map(|b|String::from_utf8(b.to_vec()).unwrap())}));
            let mut response=ResponseInput::new(status,vec![Header::new("content-type","application/json")],Arc::from(body.as_bytes()), BodyState::Complete, Provenance::Wire);response.header_delivery=HeaderDelivery::Complete;
            TransportResult{upload:UploadState::Complete,response:Some(response),error_detail:None}
        }).await
        })
    }
}
pub async fn binding_call<P: binding::BindingInvoker>(
    p: P,
    src: &Source,
    path: &str,
    method: &str,
    context: Option<&JsonValue>,
    inputs: &[&str],
) -> Result<Vec<String>, InvocationError> {
    let b = BindingValue {
        content: Some(value(json!({"path":path,"method":method}))),
        ..Default::default()
    };
    let invoker = binding::Invoker::new(p);
    let mut call = invoker.invoke(BindingRequest {
        source: src,
        binding: &b,
        context,
    });
    for v in inputs {
        let _ = call.write(JsonValue::parse(v).unwrap()).await;
    }
    call.close();
    drain(&mut call).await
}
async fn drain<F: Future<Output = Result<(), InvocationError>>>(
    call: &mut openbindings_invocation::Invocation<F>,
) -> Result<Vec<String>, InvocationError> {
    let mut out = vec![];
    while let Some(v) = call.next().await {
        match v {
            Ok(v) => out.push(v.text().into()),
            Err(e) => {
                assert!(out.is_empty());
                return Err(e);
            }
        }
    }
    call.outcome().expect("terminal")?;
    Ok(out)
}
pub async fn operation_call<P: binding::BindingInvoker>(
    p: P,
    saved: &str,
    key: &str,
    context: &JsonValue,
    input: &str,
) -> Result<Vec<String>, InvocationError> {
    let invoker = operation::Invoker::new(
        Document::parse(saved)
            .unwrap()
            .validate()
            .unwrap()
            .conformant()
            .expect("conformant invocation document"),
        p,
    )
    .with_evaluator(
        Arc::new(openbindings_json_schema_evaluator::DefaultEvaluator::new()),
        openbindings::ResourceSet::default(),
    );
    let mut call = invoker.invoke(operation::Request::operation(key).with_context(context));
    let _ = call.write(JsonValue::parse(input).unwrap()).await;
    call.close();
    drain(&mut call).await
}
fn assert_wire(seen: &Seen, row: &Value) {
    assert_eq!(seen.calls.get(), 1);
    let wires = seen.wires.borrow();
    assert_eq!(wires.len(), 1);
    let w = &wires[0];
    assert_eq!(
        w["target"],
        format!("https://api.example.test{}", row["path"].as_str().unwrap())
    );
    assert_eq!(w["body"], row["bodyText"]);
    let h = w["headers"].as_array().unwrap();
    for name in ["authorization", "x-partner-key", "x-second-key"] {
        let matches: Vec<_> = h.iter().filter(|x| x[0] == name).collect();
        let expected = row["authHeaders"]
            .as_object()
            .unwrap()
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v);
        assert_eq!(matches.len(), usize::from(expected.is_some()));
        if let Some(expected) = expected {
            assert_eq!(&matches[0][1], expected);
        }
    }
}
pub async fn suite(v30: bool) {
    let (kind, text, manual, variants) = edition(v30);
    let src = source(kind, text);
    let point = format!("{kind}.security");
    let oracle: Value =
        crate::wire_json::from_str(include_str!("auth-fixtures/wire-oracle.json")).unwrap();
    let inspector = OpenApi::default();
    assert_eq!(
        openbindings_source_inspection::Inspector::new(&inspector)
            .inspect(&src)
            .await
            .unwrap()
            .targets
            .len(),
        7
    );
    let sources = [SynthesisSource {
        name: Some("api"),
        source: &src,
    }];
    let synthesis = Synthesizer::new(inspector)
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &sources,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        synthesis
            .document()
            .as_document()
            .value()
            .get("operations")
            .unwrap()
            .members()
            .unwrap()
            .count(),
        6
    );
    let mut generated = synthesis.into_parts().0.into_inner();
    // Rename operations and their binding references atomically; keep all other bytes.
    let mut operations = std::collections::BTreeMap::new();
    let mut replacements = Vec::new();
    for (index, binding) in generated
        .bindings()
        .unwrap()
        .unwrap()
        .into_iter()
        .enumerate()
    {
        let old = binding.operation().unwrap();
        let new = format!("renamed_{index}");
        let operation = generated
            .value()
            .get("operations")
            .unwrap()
            .get(old)
            .unwrap()
            .to_owned();
        operations.insert(new.clone(), operation);
        let key = binding.key().replace('~', "~0").replace('/', "~1");
        replacements.push((
            format!("/bindings/{key}/operation"),
            JsonValue::from_serializable(&new).unwrap(),
        ));
    }
    replacements.push((
        "/operations".into(),
        JsonValue::parse(crate::wire_json::to_string(&operations).unwrap()).unwrap(),
    ));
    let edits: Vec<_> = replacements
        .iter()
        .map(|(pointer, value)| openbindings::Edit::Set { pointer, value })
        .collect();
    generated.edit(&edits).unwrap();
    let saved = generated.value().text().to_owned();
    for row in oracle["requests"].as_array().unwrap() {
        let names: Vec<_> = row["securitySchemes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        let mut c = context(kind, &names);
        c["credentials"] = row["credentials"].clone();
        let c = value(c);
        let path = row["path"].as_str().unwrap();
        let method = row["method"].as_str().unwrap().to_lowercase();
        let input = row["inputText"].as_str().unwrap();
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
        assert_eq!(
            binding_call(p, &src, path, &method, Some(&c), &[input])
                .await
                .unwrap(),
            [OUTPUT]
        );
        assert_wire(&seen, row);
        for doc in [manual, &saved] {
            let draft = Document::parse(doc).unwrap().clone();
            let key = &draft
                .bindings()
                .unwrap()
                .unwrap()
                .into_iter()
                .find(|b| b.content().unwrap().get("path").unwrap().as_str() == Some(path))
                .unwrap()
                .operation()
                .unwrap()
                .to_owned();
            let seen = Rc::new(Seen::default());
            let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
            assert_eq!(
                operation_call(p, doc, key, &c, input).await.unwrap(),
                [OUTPUT]
            );
            assert_wire(&seen, row);
        }
    }
    let variants: Value = crate::wire_json::from_str(variants).unwrap();
    for n in [
        "mixed-unsupported",
        "roles",
        "query",
        "cookie",
        "oauth",
        "oidc",
        "unknown",
        "mtls",
        "collision",
        "key-collision",
    ] {
        let s = source(kind, &variants[n].to_string());
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
        assert_eq!(
            binding_call(
                p,
                &s,
                "/items",
                "get",
                Some(&value(context(kind, &["ServiceToken"]))),
                &["{}"]
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED",
            "{n}"
        );
        assert_eq!(seen.calls.get(), 0);
        let input = [SynthesisSource {
            name: None,
            source: &s,
        }];
        let result = Synthesizer::new(OpenApi::default())
            .synthesize_with_coverage(&SynthesisRequest {
                sources: &input,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(result.coverage().entries().iter().any(|e|e.source_ref=="/paths/~1items/get" && e.status==openbindings_interface_synthesis::CoverageStatus::ImplementationUnsupported));
        assert!(
            result
                .document()
                .as_document()
                .value()
                .get("operations")
                .unwrap()
                .members()
                .unwrap()
                .count()
                > 0
        );
    }
    for (n, names) in [
        ("two-keys", vec!["PartnerKey", "SecondKey"]),
        ("duplicate-alternatives", vec!["ServiceToken"]),
        ("anonymous", vec![]),
        ("reordered", vec!["PartnerKey"]),
    ] {
        let s = source(kind, &variants[n].to_string());
        let mut c = context(kind, &names);
        if n == "duplicate-alternatives" {
            c["configuration"][&point]
                .as_object_mut()
                .unwrap()
                .remove("schemes");
        }
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
        assert!(
            binding_call(p, &s, "/items", "get", Some(&value(c)), &["{}"])
                .await
                .is_ok(),
            "{n}"
        );
        assert_eq!(seen.calls.get(), 1);
    }
    for (n, names) in [
        ("collision", vec!["ServiceToken", "Other"]),
        ("key-collision", vec!["PartnerKey", "Other"]),
    ] {
        let s = source(kind, &variants[n].to_string());
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
        assert_eq!(
            binding_call(
                p,
                &s,
                "/items",
                "get",
                Some(&value(context(kind, &names))),
                &["{}"]
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
        assert_eq!(seen.calls.get(), 0);
    }
    let good = context(kind, &["ServiceToken"]);
    let mut contexts: Vec<(Value, &str)> = vec![(json!({}), "CONTEXT_REQUIRED")];
    for v in [
        Value::Null,
        json!(23),
        json!({}),
        json!("bad token"),
        json!("fixture-bearer-A\r\ninject: x"),
    ] {
        let mut c = good.clone();
        c["credentials"]["ServiceToken"] = v;
        contexts.push((c, "ERR_REFUSED"));
    }
    for v in [
        json!(null),
        json!([]),
        json!([23]),
        json!(["not-a-url"]),
        json!(["https://api.example.test", "not-a-url"]),
        json!(["http://api.example.test"]),
        json!(["https://api.example.test:444"]),
        json!(["https://source.example.test"]),
    ] {
        let mut c = good.clone();
        c["configuration"][&point]["origins"]["ServiceToken"] = v;
        contexts.push((c, "ERR_REFUSED"));
    }
    for v in [
        json!(["Unknown"]),
        json!(["ServiceToken", "ServiceToken"]),
        json!(null),
        json!([]),
    ] {
        let mut c = good.clone();
        c["configuration"][&point]["schemes"] = v;
        contexts.push((c, "ERR_REFUSED"));
    }
    let mut conflict = good.clone();
    conflict["bearerToken"] = json!("fixture-CONFLICT");
    contexts.push((conflict, "ERR_REFUSED"));
    let mut missing = good.clone();
    missing["credentials"] = json!({});
    contexts.push((missing, "CONTEXT_REQUIRED"));
    let mut missing = good.clone();
    missing["configuration"][&point]
        .as_object_mut()
        .unwrap()
        .remove("origins");
    contexts.push((missing, "CONTEXT_REQUIRED"));
    let wrong_kind = if v30 {
        openbindings_openapi::EXPERIMENTAL_KIND
    } else {
        openbindings_openapi::EXPERIMENTAL_KIND_30
    };
    contexts.push((context(wrong_kind, &["ServiceToken"]), "CONTEXT_REQUIRED"));
    let seen = Rc::new(Seen::default());
    let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
    // Same provider: A then B then absent. Context is never retained in its cache.
    for token in ["fixture-bearer-A", "fixture-bearer-B"] {
        let mut c = good.clone();
        c["credentials"]["ServiceToken"] = json!(token);
        assert!(
            binding_call(&p, &src, "/items", "get", Some(&value(c)), &["{}"])
                .await
                .is_ok()
        );
    }
    let before = seen.calls.get();
    for (c, code) in contexts {
        let e = binding_call(&p, &src, "/items", "get", Some(&value(c)), &["{}"])
            .await
            .unwrap_err();
        assert_eq!(e.code(), code);
        let debug = format!("{e:?}");
        assert!(!debug.contains("fixture-"));
        assert!(!e.to_json().unwrap().text().contains("fixture-"));
        assert_eq!(seen.calls.get(), before);
    }
    for c in [
        r#"{"credentials":{"ServiceToken":"fixture-bearer-A","ServiceToken":"fixture-bearer-B"}}"#
            .to_owned(),
        format!(r#"{{"configuration":{{"{point}":{{"schemes":["ServiceToken"],"schemes":[]}}}}}}"#),
    ] {
        assert_eq!(
            binding_call(
                &p,
                &src,
                "/items",
                "get",
                Some(&JsonValue::parse(c).unwrap()),
                &["{}"]
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
        assert_eq!(seen.calls.get(), before);
    }
    // Explicit compatibility forms and canonical origins.
    for alias in ["bearerToken", "equal"] {
        let mut c = good.clone();
        c["bearerToken"] = json!("fixture-bearer-A");
        if alias != "equal" {
            c.as_object_mut().unwrap().remove("credentials");
        }
        c["configuration"][&point]["origins"]["ServiceToken"] =
            json!(["HTTPS://API.EXAMPLE.TEST:443"]);
        assert!(
            binding_call(&p, &src, "/items", "get", Some(&value(c)), &["{}"])
                .await
                .is_ok()
        );
    }
    for alias in ["apiKey", "apiKeys"] {
        let mut c = context(kind, &["PartnerKey"]);
        c.as_object_mut().unwrap().remove("credentials");
        c[alias] = if alias == "apiKey" {
            json!("fixture-key-A")
        } else {
            json!({"PartnerKey":"fixture-key-A"})
        };
        assert!(
            binding_call(
                &p,
                &src,
                "/items/new",
                "post",
                Some(&value(c)),
                &["{\"body\":null}"]
            )
            .await
            .is_ok()
        );
    }
    for path in ["/choice", "/optional"] {
        let before = seen.calls.get();
        let e = binding_call(
            &p,
            &src,
            path,
            "get",
            Some(&value(
                json!({"credentials":{"ServiceToken":"fixture-bearer-A"}}),
            )),
            &["{}"],
        )
        .await
        .unwrap_err();
        assert_eq!(e.code(), "CONTEXT_REQUIRED");
        assert_eq!(seen.calls.get(), before);
        assert!(
            binding_call(
                &p,
                &src,
                path,
                "get",
                Some(&value(context(kind, &[]))),
                &["{}"]
            )
            .await
            .is_ok()
        );
    }
    let two = source(kind, &variants["two-servers"].to_string());
    let e = binding_call(&p, &two, "/items", "get", None, &["{}"])
        .await
        .unwrap_err();
    let encoded = e.to_json().unwrap().text().to_owned();
    assert!(encoded.contains(&format!("{kind}.server")));
    assert!(!encoded.contains("auth.bearer"));
    let mut c = good.clone();
    c["configuration"][format!("{kind}.server")] = json!("https://api.example.test");
    assert!(
        binding_call(&p, &two, "/items", "get", Some(&value(c)), &["{}"])
            .await
            .is_ok()
    );
    for status in [401, 403, 302] {
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), status, OUTPUT));
        assert_eq!(
            binding_call(
                p,
                &src,
                "/items",
                "get",
                Some(&value(good.clone())),
                &["{}"]
            )
            .await
            .unwrap_err()
            .code(),
            format!("{kind}.http-status")
        );
        assert_eq!(seen.calls.get(), 1);
    }
    let seen = Rc::new(Seen::default());
    let p = OpenApi::default().with_transport(host(seen.clone(), 200, "{"));
    assert_eq!(
        binding_call(
            p,
            &src,
            "/items",
            "get",
            Some(&value(good.clone())),
            &["{}"]
        )
        .await
        .unwrap_err()
        .code(),
        "ERR_EXECUTION_FAILED"
    );
    assert_eq!(seen.calls.get(), 1);
    for inputs in [vec![], vec!["{}", "{}"]] {
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
        assert_eq!(
            binding_call(
                p,
                &src,
                "/items",
                "get",
                Some(&value(good.clone())),
                &inputs
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
        assert_eq!(seen.calls.get(), 0);
    }
    let c = value(good.clone());
    for (limit, ok) in [(c.bytes().len(), true), (c.bytes().len() - 1, false)] {
        let mut o = Options::default();
        o.context_bytes = limit;
        let seen = Rc::new(Seen::default());
        let p = OpenApi::new(o)
            .unwrap()
            .with_transport(host(seen.clone(), 200, OUTPUT));
        assert_eq!(
            binding_call(p, &src, "/items", "get", Some(&c), &["{}"])
                .await
                .is_ok(),
            ok
        );
        assert_eq!(seen.calls.get(), usize::from(ok));
    }
    // External resource names are resolved in the entry OpenAPI document in this experiment.
    let mut o = Options::default();
    o.resources.push(
        openbindings_openapi::ResourceInput::at(
            "https://source.example.test/parts.json",
            if v30 {
                include_bytes!("auth-fixtures/parts-3.0.4.json").as_slice()
            } else {
                include_bytes!("auth-fixtures/parts-3.1.2.json").as_slice()
            },
        )
        .unwrap(),
    );
    let s = source(kind, &variants["references"].to_string());
    let reference_oracle = &oracle["requests"][0];
    let seen = Rc::new(Seen::default());
    let p = OpenApi::new(o.clone())
        .unwrap()
        .with_transport(host(seen.clone(), 200, OUTPUT));
    assert!(
        binding_call(p, &s, "/items", "get", Some(&c), &["{}"])
            .await
            .is_ok()
    );
    assert_wire(&seen, reference_oracle);
    assert!(
        !crate::wire_json::to_string(&*seen.wires.borrow())
            .unwrap()
            .contains("x-wrong-secondary")
    );
    let sources = [SynthesisSource {
        name: Some("api"),
        source: &s,
    }];
    let result = Synthesizer::new(OpenApi::new(o.clone()).unwrap())
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &sources,
            ..Default::default()
        })
        .await
        .unwrap();
    let saved = result.document().as_document().value().text();
    let draft = result.document().as_document();
    let name = &draft
        .bindings()
        .unwrap()
        .unwrap()
        .into_iter()
        .find(|b| b.content().unwrap().get("path").unwrap().as_str() == Some("/items"))
        .unwrap()
        .operation()
        .unwrap()
        .to_owned();
    let seen = Rc::new(Seen::default());
    assert!(
        operation_call(
            OpenApi::new(o)
                .unwrap()
                .with_transport(host(seen.clone(), 200, OUTPUT)),
            saved,
            name,
            &c,
            "{}"
        )
        .await
        .is_ok()
    );
    assert_wire(&seen, reference_oracle);
    assert!(
        !crate::wire_json::to_string(&*seen.wires.borrow())
            .unwrap()
            .contains("x-wrong-secondary")
    );
    let seen = Rc::new(Seen::default());
    assert_eq!(
        binding_call(
            OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT)),
            &s,
            "/items",
            "get",
            Some(&c),
            &["{}"]
        )
        .await
        .unwrap_err()
        .code(),
        "ERR_REFUSED"
    );
    assert_eq!(seen.calls.get(), 0);
}

/// Repeated resource/effect mix including a pending host and independent sibling.
pub async fn cycle(v30: bool) {
    use openbindings_source_inspection::CancellationToken;
    use std::task::Poll;
    let (kind, text, _, _) = edition(v30);
    let src = source(kind, text);
    let good = value(context(kind, &["ServiceToken"]));
    for (ctx, status, body, code, effects) in [
        (Some(&good), 200, OUTPUT, None, 1),
        (None, 200, OUTPUT, Some("CONTEXT_REQUIRED"), 0),
        (
            Some(&good),
            401,
            OUTPUT,
            Some(if v30 {
                openbindings_openapi::HTTP_STATUS_ERROR_30
            } else {
                openbindings_openapi::HTTP_STATUS_ERROR
            }),
            1,
        ),
        (Some(&good), 200, "{", Some("ERR_EXECUTION_FAILED"), 1),
    ] {
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), status, body));
        let r = binding_call(p, &src, "/items", "get", ctx, &["{}"]).await;
        match code {
            None => assert_eq!(r.unwrap(), [OUTPUT]),
            Some(code) => assert_eq!(r.unwrap_err().code(), code),
        }
        assert_eq!(seen.calls.get(), effects);
    }
    let mut wrong = context(kind, &["ServiceToken"]);
    wrong["configuration"][format!("{kind}.security")]["origins"]["ServiceToken"] =
        json!(["https://wrong.test"]);
    let seen = Rc::new(Seen::default());
    assert_eq!(
        binding_call(
            OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT)),
            &src,
            "/items",
            "get",
            Some(&value(wrong)),
            &["{}"]
        )
        .await
        .unwrap_err()
        .code(),
        "ERR_REFUSED"
    );
    assert_eq!(seen.calls.get(), 0);
    struct Guard(Rc<Cell<usize>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }
    for abandon in [false, true] {
        let live = Rc::new(Cell::new(0));
        let effects = Rc::new(Cell::new(0));
        let count = effects.clone();
        let registered = live.clone();
        let p = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| {
                let registered = registered.clone();
                let count = count.clone();
                async move {
                    count.set(count.get() + 1);
                    let pending = request
                        .headers()
                        .iter()
                        .any(|h| h.value() == b"Bearer fixture-bearer-A");
                    if pending {
                        registered.set(registered.get() + 1);
                        let _guard = Guard(registered);
                        std::future::pending::<Outcome>().await
                    } else {
                        request
                            .invoke(cancel, &HostCapabilities::programmable(), |_, _| async {
                                let mut response = ResponseInput::new(
                                    200,
                                    vec![Header::new("content-type", "application/json")],
                                    Arc::from(OUTPUT.as_bytes()),
                                    BodyState::Complete,
                                    Provenance::Wire,
                                );
                                response.header_delivery = HeaderDelivery::Complete;
                                TransportResult {
                                    upload: UploadState::Complete,
                                    response: Some(response),
                                    error_detail: None,
                                }
                            })
                            .await
                    }
                }
            },
        );
        let invoker = binding::Invoker::new(&p);
        let b = BindingValue {
            content: Some(value(json!({"path":"/items","method":"get"}))),
            ..Default::default()
        };
        let control = CancellationToken::new();
        let mut job = invoker.invoke_with_control(
            BindingRequest {
                source: &src,
                binding: &b,
                context: Some(&good),
            },
            &control,
        );
        job.write(JsonValue::parse("{}").unwrap()).await.unwrap();
        job.close();
        {
            let mut next = std::pin::pin!(job.next());
            std::future::poll_fn(|cx| {
                assert!(next.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
        }
        assert_eq!(live.get(), 1);
        assert_eq!(effects.get(), 1);
        let mut sibling = context(kind, &["ServiceToken"]);
        sibling["credentials"]["ServiceToken"] = json!("fixture-bearer-B");
        assert!(
            binding_call(&p, &src, "/items", "get", Some(&value(sibling)), &["{}"])
                .await
                .is_ok()
        );
        assert_eq!(live.get(), 1);
        assert_eq!(effects.get(), 2);
        if !abandon {
            control.cancel();
            assert_eq!(
                job.next().await.unwrap().unwrap_err().code(),
                "ERR_CANCELLED"
            );
        }
        drop(job);
        assert_eq!(live.get(), 0);
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let mut before = invoker.invoke_with_control(
            BindingRequest {
                source: &src,
                binding: &b,
                context: Some(&good),
            },
            &cancelled,
        );
        before.close();
        assert_eq!(
            before.next().await.unwrap().unwrap_err().code(),
            "ERR_CANCELLED"
        );
        assert_eq!(effects.get(), 2);
    }
}

pub async fn bounds(v30: bool) {
    let (kind, text, _, _) = edition(v30);
    let src = source(kind, text);
    let context = value(context(kind, &["ServiceToken"]));
    // Exact retained metadata budget is measured from the observable acceptance edge.
    let mut lo = 0;
    let mut hi = 65536;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let mut o = Options::default();
        o.result_bytes = mid;
        let p = OpenApi::new(o).unwrap();
        if openbindings_source_inspection::Inspector::new(p)
            .inspect(&src)
            .await
            .is_ok()
        {
            hi = mid
        } else {
            lo = mid + 1
        }
    }
    assert!(lo > 0 && lo < 65536);
    for (n, ok) in [(lo, true), (lo - 1, false)] {
        let mut o = Options::default();
        o.result_bytes = n;
        assert_eq!(
            openbindings_source_inspection::Inspector::new(OpenApi::new(o).unwrap())
                .inspect(&src)
                .await
                .is_ok(),
            ok
        );
    }
    // JSON response/input has no transport header relevance; count exactly the request headers.
    let seen = Rc::new(Seen::default());
    let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
    binding_call(p, &src, "/items", "get", Some(&context), &["{}"])
        .await
        .unwrap();
    let count = seen.wires.borrow()[0]["headers"].as_array().unwrap().len();
    for (n, ok) in [(count, true), (count - 1, false)] {
        let mut o = Options::default();
        o.limits.headers = n;
        let seen = Rc::new(Seen::default());
        let p = OpenApi::new(o)
            .unwrap()
            .with_transport(host(seen.clone(), 200, OUTPUT));
        let outcome = binding_call(p, &src, "/items", "get", Some(&context), &["{}"]).await;
        if ok {
            assert!(outcome.is_ok());
        } else {
            assert_eq!(outcome.unwrap_err().code(), "ERR_REFUSED");
        }
        assert_eq!(seen.calls.get(), usize::from(ok));
    }
    let e = binding_call(
        OpenApi::default().with_transport(host(Rc::new(Seen::default()), 200, OUTPUT)),
        &src,
        "/items",
        "get",
        None,
        &["{}"],
    )
    .await
    .unwrap_err();
    let n = e.to_json().unwrap().bytes().len();
    assert!(n >= 256);
    for (limit, code) in [(n, "CONTEXT_REQUIRED"), (n - 1, "ERR_EXECUTION_FAILED")] {
        let seen = Rc::new(Seen::default());
        let p = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));
        let invoker = binding::Invoker::with_limits(
            p,
            openbindings_invocation::InvocationLimits {
                max_error_bytes: limit,
                ..Default::default()
            },
        )
        .unwrap();
        let b = BindingValue {
            content: Some(value(json!({"path":"/items","method":"get"}))),
            ..Default::default()
        };
        let mut call = invoker.invoke(BindingRequest {
            source: &src,
            binding: &b,
            context: None,
        });
        let _ = call.write(JsonValue::parse("{}").unwrap()).await;
        call.close();
        assert_eq!(drain(&mut call).await.unwrap_err().code(), code);
        assert_eq!(seen.calls.get(), 0);
    }
}

pub async fn additional_admission(v30: bool) {
    let (kind, text, _, _) = edition(v30);
    let mut base: Value = crate::wire_json::from_str(text).unwrap();
    for header in ["X Partner Key", "", "content-TYPE"] {
        let mut doc = base.clone();
        doc["components"]["securitySchemes"]["PartnerKey"]["name"] = json!(header);
        let src = source(kind, &doc.to_string());
        let sources = [SynthesisSource {
            name: None,
            source: &src,
        }];
        let s = Synthesizer::new(OpenApi::default())
            .synthesize_with_coverage(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(s.coverage().entries().iter().any(|e|e.source_ref=="/paths/~1items~1new/post" && e.status==openbindings_interface_synthesis::CoverageStatus::ImplementationUnsupported));
    }
    base["components"]["securitySchemes"]["SecondToken"] = json!({"type":"http","scheme":"bearer"});
    base["paths"]["/items"]["get"]["security"] = json!([{"ServiceToken":[],"SecondToken":[]}]);
    let src = source(kind, &base.to_string());
    let inspection = openbindings_source_inspection::Inspector::new(OpenApi::default())
        .inspect(&src)
        .await
        .unwrap();
    assert!(
        inspection
            .targets
            .iter()
            .find(|t| t.source_ref == "/paths/~1items/get")
            .unwrap()
            .operation
            .is_none()
    );
    let src = source(kind, text);
    let point = format!("{kind}.security");
    for token in ["", " ", "bad\r\nheader"] {
        let mut c = context(kind, &["PartnerKey"]);
        c["credentials"]["PartnerKey"] = json!(token);
        let seen = Rc::new(Seen::default());
        let result = binding_call(
            OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT)),
            &src,
            "/items/new",
            "post",
            Some(&value(c)),
            &["{\"body\":null}"],
        )
        .await;
        if token.is_empty() {
            assert!(result.is_ok());
            assert_eq!(seen.calls.get(), 1);
        } else {
            assert_eq!(result.unwrap_err().code(), "ERR_REFUSED");
            assert_eq!(seen.calls.get(), 0);
        }
    }
    let e = binding_call(
        OpenApi::default().with_transport(host(Rc::new(Seen::default()), 200, OUTPUT)),
        &src,
        "/choice",
        "get",
        None,
        &["{}"],
    )
    .await
    .unwrap_err();
    let error = e.to_json().unwrap();
    let alternatives = error.get("data").unwrap().get("alternatives").unwrap();
    assert_eq!(alternatives.elements().unwrap().count(), 3);
    for alt in alternatives.elements().unwrap() {
        let r = alt
            .get("requirements")
            .unwrap()
            .elements()
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(r.get("point").unwrap().as_str(), Some(point.as_str()));
        let schema = r.get("schema").unwrap();
        if schema.get("const").is_none() {
            assert_eq!(schema.get("uniqueItems").unwrap().as_bool(), Some(true));
            assert_eq!(
                schema.get("minItems").unwrap().number_text(),
                schema.get("maxItems").unwrap().number_text()
            );
        }
    }
}
