//! Literal acceptance assertions added by the final independent case audit.
use crate::{
    auth::*,
    wire_json::{Value, json},
};
use dynamic_openapi_client::{
    Cancellation, Header,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use openbindings::{Document, JsonValue, Source};
use openbindings_interface_synthesis::{
    CoverageScope, CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_invocation::{BindingRequest, BindingValue, binding};
use openbindings_openapi::{OpenApi, Options, ResourceInput};
use std::{rc::Rc, sync::Arc};

fn header(seen: &Seen, name: &str, expected: &str) {
    let wires = seen.wires.borrow();
    let values: Vec<_> = wires.last().unwrap()["headers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v[0] == name)
        .collect();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0][1], expected);
}
fn reset(seen: &Seen) {
    seen.calls.set(0);
    seen.wires.borrow_mut().clear();
}
async fn authored(src: &Source) -> (String, openbindings_interface_synthesis::SynthesisCoverage) {
    let inputs = [SynthesisSource {
        name: Some("api"),
        source: src,
    }];
    let output = Synthesizer::new(OpenApi::default())
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        })
        .await
        .unwrap();
    (
        output.document().parsed().value().text().to_owned(),
        output.coverage().clone(),
    )
}
pub async fn suite(v30: bool) {
    let (kind, text, _, variants) = edition(v30);
    let variants: Value = crate::wire_json::from_str(variants).unwrap();
    let src = source(kind, text);
    let point = format!("{kind}.security");
    let seen = Rc::new(Seen::default());
    let provider = OpenApi::default().with_transport(host(seen.clone(), 200, OUTPUT));

    // AJ03/04: every conjunct is needed; order is immaterial; two keys reach distinct fields.
    for (s, path, method, names, input) in [
        (
            src.clone(),
            "/combined",
            "post",
            vec!["ServiceToken", "PartnerKey"],
            r#"{"body":null}"#,
        ),
        (
            source(kind, &variants["two-keys"].to_string()),
            "/items",
            "get",
            vec!["PartnerKey", "SecondKey"],
            "{}",
        ),
    ] {
        for ordered in [names.clone(), names.iter().rev().copied().collect()] {
            reset(&seen);
            assert_eq!(
                binding_call(
                    &provider,
                    &s,
                    path,
                    method,
                    Some(&value(context(kind, &ordered))),
                    &[input]
                )
                .await
                .unwrap(),
                [OUTPUT]
            );
            assert_eq!(seen.calls.get(), 1);
            header(&seen, "x-partner-key", "fixture-key-A");
            header(
                &seen,
                if names.contains(&"ServiceToken") {
                    "authorization"
                } else {
                    "x-second-key"
                },
                if names.contains(&"ServiceToken") {
                    "Bearer fixture-bearer-A"
                } else {
                    "fixture-key-A"
                },
            );
        }
        for name in &names {
            for omit_grant in [false, true] {
                let mut c = context(kind, &names);
                let map = if omit_grant {
                    &mut c["configuration"][&point]["origins"]
                } else {
                    &mut c["credentials"]
                };
                map.as_object_mut().unwrap().remove(*name);
                reset(&seen);
                let e = binding_call(&provider, &s, path, method, Some(&value(c)), &[input])
                    .await
                    .unwrap_err();
                assert_eq!(e.code(), "CONTEXT_REQUIRED");
                assert_eq!(seen.calls.get(), 0);
                assert!(seen.wires.borrow().is_empty());
            }
        }
    }
    // Select a branch whose authored index actually changes (0 -> 2).
    for s in [&src, &source(kind, &variants["reordered"].to_string())] {
        reset(&seen);
        let path = if std::ptr::eq(s, &src) {
            "/choice"
        } else {
            "/items"
        };
        binding_call(
            &provider,
            s,
            path,
            "get",
            Some(&value(context(kind, &["ServiceToken"]))),
            &["{}"],
        )
        .await
        .unwrap();
        header(&seen, "authorization", "Bearer fixture-bearer-A");
        assert_eq!(seen.calls.get(), 1);
    }
    reset(&seen);
    let mut both = context(kind, &["ServiceToken", "PartnerKey"]);
    both["configuration"][&point]
        .as_object_mut()
        .unwrap()
        .remove("schemes");
    assert_eq!(
        binding_call(
            &provider,
            &src,
            "/choice",
            "get",
            Some(&value(both)),
            &["{}"]
        )
        .await
        .unwrap_err()
        .code(),
        "CONTEXT_REQUIRED"
    );
    assert_eq!(seen.calls.get(), 0);

    // AJ06: challenge details and a deliberate new attempt leave the old call terminal.
    let b = BindingValue {
        content: Some(value(json!({"path":"/items","method":"get"}))),
        ..Default::default()
    };
    let facade = binding::Invoker::new(&provider);
    let mut failed = facade.invoke(BindingRequest {
        source: &src,
        binding: &b,
        context: None,
    });
    failed.write(JsonValue::parse("{}").unwrap()).await.unwrap();
    failed.close();
    let e = failed.next().await.unwrap().unwrap_err();
    assert_eq!(e.code(), "CONTEXT_REQUIRED");
    let challenge: Value = crate::wire_json::from_str(e.data().unwrap().text()).unwrap();
    let requirements = challenge["alternatives"][0]["requirements"]
        .as_array()
        .unwrap();
    assert!(
        requirements
            .iter()
            .any(|r| r["type"] == "auth.bearer" && r["name"] == "ServiceToken")
    );
    assert!(requirements.iter().any(|r| r["type"] == "config.value"
        && r["point"] == point
        && r["path"] == "/origins/ServiceToken"));
    assert_eq!(seen.calls.get(), 0);
    assert!(failed.next().await.is_none());
    binding_call(
        &provider,
        &src,
        "/items",
        "get",
        Some(&value(context(kind, &["ServiceToken"]))),
        &["{}"],
    )
    .await
    .unwrap();
    assert_eq!(seen.calls.get(), 1);
    assert!(failed.next().await.is_none());
    assert_eq!(
        failed.outcome().unwrap().unwrap_err().code(),
        "CONTEXT_REQUIRED"
    );

    // AJ07/08: a chosen destination never inherits another destination's grant.
    let two = source(kind, &variants["two-servers"].to_string());
    reset(&seen);
    let e = binding_call(&provider, &two, "/items", "get", None, &["{}"])
        .await
        .unwrap_err();
    assert_eq!(e.code(), "CONTEXT_REQUIRED");
    assert_eq!(seen.calls.get(), 0);
    assert!(!e.to_json().unwrap().text().contains("auth.bearer"));
    let mut c = context(kind, &["ServiceToken"]);
    let server_point = format!("{kind}.server");
    c["configuration"][&server_point] = json!("https://api.example.test");
    binding_call(
        &provider,
        &two,
        "/items",
        "get",
        Some(&value(c.clone())),
        &["{}"],
    )
    .await
    .unwrap();
    assert_eq!(
        seen.wires.borrow()[0]["target"],
        "https://api.example.test/items"
    );
    c["configuration"][&server_point] = json!("https://second.example.test");
    reset(&seen);
    assert_eq!(
        binding_call(
            &provider,
            &two,
            "/items",
            "get",
            Some(&value(c.clone())),
            &["{}"]
        )
        .await
        .unwrap_err()
        .code(),
        "ERR_REFUSED"
    );
    assert_eq!(seen.calls.get(), 0);
    let selected_only =
        value(json!({"configuration":{&server_point:"https://second.example.test"}}));
    let e = binding_call(
        &provider,
        &two,
        "/items",
        "get",
        Some(&selected_only),
        &["{}"],
    )
    .await
    .unwrap_err();
    assert_eq!(e.code(), "CONTEXT_REQUIRED");
    assert_eq!(seen.calls.get(), 0);
    assert!(e.to_json().unwrap().text().contains("auth.bearer"));
    c["configuration"][&point]["origins"]["ServiceToken"] = json!(["https://second.example.test"]);
    binding_call(&provider, &two, "/items", "get", Some(&value(c)), &["{}"])
        .await
        .unwrap();
    assert_eq!(seen.calls.get(), 1);
    assert_eq!(
        seen.wires.borrow()[0]["target"],
        "https://second.example.test/items"
    );
    header(&seen, "authorization", "Bearer fixture-bearer-A");

    // AJ09: alternate kinds and source bytes through one provider, checking actual credentials.
    for (index, (other, token)) in [
        (v30, "fixture-bearer-A"),
        (!v30, "fixture-bearer-B"),
        (v30, "fixture-bearer-C"),
    ]
    .into_iter()
    .enumerate()
    {
        let (k, t, _, _) = edition(other);
        let mut d: Value = crate::wire_json::from_str(t).unwrap();
        d["info"]["title"] = json!(format!("public-source-{index}"));
        let s = source(k, &d.to_string());
        let mut c = context(k, &["ServiceToken"]);
        c["credentials"]["ServiceToken"] = json!(token);
        reset(&seen);
        binding_call(&provider, &s, "/items", "get", Some(&value(c)), &["{}"])
            .await
            .unwrap();
        header(&seen, "authorization", &format!("Bearer {token}"));
        assert_eq!(seen.calls.get(), 1);
        assert_eq!(
            binding_call(&provider, &s, "/items", "get", None, &["{}"])
                .await
                .unwrap_err()
                .code(),
            "CONTEXT_REQUIRED"
        );
        assert_eq!(seen.calls.get(), 1);
    }

    // AJ10: unavailable mixed-edition resources and ambiguous mounted declarations cannot dispatch.
    for ambiguous in [false, true] {
        let mut d = variants["references"].clone();
        let parts = if v30 {
            include_str!("auth-fixtures/parts-3.0.4.json")
        } else {
            include_str!("auth-fixtures/parts-3.1.2.json")
        };
        let mut parts: Value = crate::wire_json::from_str(parts).unwrap();
        if ambiguous {
            d["paths"]["/items"]["get"] = json!({"security":[],"responses":{"200":{"description":"conflicting local operation"}}});
        } else {
            parts["openapi"] = json!(if v30 { "3.1.2" } else { "3.0.4" });
        }
        let mut options = Options::default();
        options.resources.push(
            ResourceInput::at(
                "https://source.example.test/parts.json",
                parts.to_string().as_bytes(),
            )
            .unwrap(),
        );
        let p = OpenApi::new(options)
            .unwrap()
            .with_transport(host(seen.clone(), 200, OUTPUT));
        reset(&seen);
        assert_eq!(
            binding_call(
                p,
                &source(kind, &d.to_string()),
                "/items",
                "get",
                Some(&value(context(kind, &["ServiceToken"]))),
                &["{}"]
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
        assert_eq!(seen.calls.get(), 0);
    }

    // AJ11/12: API-key roles and every conflicting compatibility carrier.
    let mut d: Value = crate::wire_json::from_str(text).unwrap();
    d["paths"]["/items/new"]["post"]["security"] = json!([{"PartnerKey":["writer"]}]);
    let (_, role_coverage) = authored(&source(kind, &d.to_string())).await;
    assert!(
        role_coverage
            .entries()
            .iter()
            .any(|e| e.source_ref == "/paths/~1items~1new/post"
                && e.status == CoverageStatus::ImplementationUnsupported
                && e.operation_key.is_none())
    );
    reset(&seen);
    assert_eq!(
        binding_call(
            &provider,
            &source(kind, &d.to_string()),
            "/items/new",
            "post",
            Some(&value(context(kind, &["PartnerKey"]))),
            &[r#"{"body":null}"#]
        )
        .await
        .unwrap_err()
        .code(),
        "ERR_REFUSED"
    );
    assert_eq!(seen.calls.get(), 0);
    for (alias, supplied) in [
        ("apiKey", json!("fixture-conflicting-key")),
        ("apiKeys", json!({"PartnerKey":"fixture-conflicting-key"})),
    ] {
        let mut c = context(kind, &["PartnerKey"]);
        c[alias] = supplied;
        assert_eq!(
            binding_call(
                &provider,
                &src,
                "/items/new",
                "post",
                Some(&value(c)),
                &[r#"{"body":null}"#]
            )
            .await
            .unwrap_err()
            .code(),
            "ERR_REFUSED"
        );
        assert_eq!(seen.calls.get(), 0);
    }

    // AJ13/17: preserve opaque source fields and keep security prerequisites separate from schema loss.
    let mut preserved = src.clone();
    preserved.additional_fields.insert(
        "x-authoring-style".into(),
        value(json!({"n":9007199254740993u64,"label":"雪"})),
    );
    let (saved, coverage) = authored(&preserved).await;
    let parsed = openbindings::ParsedDocument::parse(&saved).unwrap();
    let draft = Document::from_json(parsed.value()).unwrap();
    let output_source = &draft.sources.as_ref().unwrap()["api"];
    assert_eq!(
        output_source.content.as_ref().unwrap().text(),
        preserved.content.as_ref().unwrap().text()
    );
    assert_eq!(
        output_source.additional_fields["x-authoring-style"].text(),
        preserved.additional_fields["x-authoring-style"].text()
    );
    for (reference, sizes) in [
        ("/paths/~1combined/post", vec![2]),
        ("/paths/~1choice/get", vec![1, 1, 0]),
    ] {
        let e = coverage
            .entries()
            .iter()
            .find(|e| e.source_ref == reference && e.status == CoverageStatus::Represented)
            .unwrap();
        assert_eq!(
            e.requirements.as_ref().unwrap().as_slice(),
            std::slice::from_ref(&point)
        );
        let details: Value =
            crate::wire_json::from_str(e.details.as_ref().unwrap()["security"].text()).unwrap();
        assert_eq!(
            details["alternatives"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_array().unwrap().len())
                .collect::<Vec<_>>(),
            sizes
        );
        assert!(coverage.entries().iter().any(|e| e.source_ref == reference
            && e.scope == CoverageScope::Projection
            && e.status == CoverageStatus::Lossy));
    }
    let coverage_text = crate::wire_json::to_string(&coverage).unwrap();
    for output in [saved.as_str(), coverage_text.as_str()] {
        assert!(!output.contains("fixture-bearer-"));
        assert!(!output.contains("fixture-key-"));
    }
    let mut no_schema: Value = crate::wire_json::from_str(text).unwrap();
    no_schema["paths"]["/items"]["get"]["responses"]["200"]["content"]["application/json"]
        .as_object_mut()
        .unwrap()
        .remove("schema");
    let (_, no_schema_coverage) = authored(&source(kind, &no_schema.to_string())).await;
    let target = "/paths/~1items/get";
    let represented = no_schema_coverage
        .entries()
        .iter()
        .find(|e| e.source_ref == target && e.status == CoverageStatus::Represented)
        .unwrap();
    assert_eq!(
        represented.requirements.as_ref().unwrap().as_slice(),
        std::slice::from_ref(&point)
    );
    assert!(
        !no_schema_coverage
            .entries()
            .iter()
            .any(|e| e.source_ref == target
                && e.scope == CoverageScope::Projection
                && e.status == CoverageStatus::Lossy)
    );

    // AJ14: valid JSON with explicit incomplete delivery evidence is still execution failure.
    let count = seen.clone();
    let p =
        OpenApi::default().with_transport(move |request: PreparedRequest, cancel: Cancellation| {
            let count = count.clone();
            async move {
                count.calls.set(count.calls.get() + 1);
                request
                    .invoke(cancel, &HostCapabilities::programmable(), |_, _| async {
                        let mut response = ResponseInput::new(
                            200,
                            vec![Header::new("content-type", "application/json")],
                            Arc::from(OUTPUT.as_bytes()),
                            BodyState::Truncated,
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
        });
    reset(&seen);
    assert_eq!(
        binding_call(
            p,
            &src,
            "/items",
            "get",
            Some(&value(context(kind, &["ServiceToken"]))),
            &["{}"]
        )
        .await
        .unwrap_err()
        .code(),
        "ERR_EXECUTION_FAILED"
    );
    assert_eq!(seen.calls.get(), 1);

    // AJ16: a large credential reaches the exact encoded context ceiling; one more byte refuses.
    let mut c = context(kind, &["PartnerKey"]);
    c["credentials"]["PartnerKey"] = json!("");
    let overhead = value(c.clone()).bytes().len();
    for extra in [0, 1] {
        c["credentials"]["PartnerKey"] = json!("a".repeat(65536 - overhead + extra));
        let c = value(c.clone());
        assert_eq!(c.bytes().len(), 65536 + extra);
        reset(&seen);
        let outcome = binding_call(
            &provider,
            &src,
            "/items/new",
            "post",
            Some(&c),
            &[r#"{"body":null}"#],
        )
        .await;
        if extra == 0 {
            assert_eq!(outcome.unwrap(), [OUTPUT]);
        } else {
            assert_eq!(outcome.unwrap_err().code(), "ERR_REFUSED");
        }
        assert_eq!(seen.calls.get(), usize::from(extra == 0));
    }
}
