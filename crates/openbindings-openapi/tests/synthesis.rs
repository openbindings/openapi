use openbindings::{Document, JsonValue, Source};
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisErrorKind, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_openapi::{EXPERIMENTAL_KIND, OpenApi, Options, ResourceInput};
use openbindings_source_inspection::CancellationToken;
use openbindings_source_inspection::Inspector;
fn source(text: &str) -> Source {
    Source{kind:EXPERIMENTAL_KIND.into(),content:Some(JsonValue::parse(serde_json_compat::json!({"document":text,"retrievalUri":"https://descriptions.example.invalid/root.json"}).to_string()).unwrap()),..Default::default()}
}
#[tokio::test]
async fn skeleton_preservation_and_composition_boundaries() {
    let provider = OpenApi::default();
    let facade = Synthesizer::new(&provider);
    let skeleton = facade
        .synthesize(&SynthesisRequest {
            name: Some("Empty"),
            version: Some("v1"),
            description: Some("Skeleton"),
            ..Default::default()
        })
        .await
        .unwrap();
    let draft = Document::from_json(skeleton.parsed().value()).unwrap();
    assert!(draft.operations.is_empty());
    assert!(draft.sources.is_none());
    assert_eq!(draft.name.as_deref(), Some("Empty"));
    let input = source(include_str!("fixtures/r1/parcels.json"));
    let sources = [SynthesisSource {
        name: Some("user_source"),
        source: &input,
    }];
    let request = SynthesisRequest {
        sources: &sources,
        ..Default::default()
    };
    let result = facade.synthesize_with_coverage(&request).await.unwrap();
    assert!(result.coverage().strict_eligible());
    assert!(!result.coverage().fully_represented());
    let draft = Document::from_json(result.document().parsed().value()).unwrap();
    assert_eq!(
        draft.sources.as_ref().unwrap()["user_source"]
            .content
            .as_ref()
            .unwrap()
            .text(),
        input.content.as_ref().unwrap().text()
    );
    let two = [sources[0], sources[0]];
    assert_eq!(
        facade
            .synthesize(&SynthesisRequest {
                sources: &two,
                ..Default::default()
            })
            .await
            .unwrap_err()
            .kind(),
        SynthesisErrorKind::UnsupportedRequest
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        facade
            .synthesize_with_control(&request, &cancelled)
            .await
            .unwrap_err()
            .kind(),
        SynthesisErrorKind::Cancelled
    );
}
#[tokio::test]
async fn missing_resource_remains_unknown_and_known_gaps_are_not_exclusions() {
    let source = source(include_str!("fixtures/r1/mounted.json"));
    let inputs = [SynthesisSource {
        name: None,
        source: &source,
    }];
    let provider = OpenApi::default();
    let facade = Synthesizer::new(&provider);
    let result = facade
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!result.coverage().exhaustive());
    assert!(result.coverage().limitation().is_some());
    assert_eq!(
        facade
            .synthesize(&SynthesisRequest {
                sources: &inputs,
                ..Default::default()
            })
            .await
            .unwrap_err()
            .kind(),
        SynthesisErrorKind::Incomplete
    );
    let mut api: serde_json_compat::Value =
        serde_json_compat::from_str(include_str!("fixtures/r1/parcels.json")).unwrap();
    api["paths"]["/parcels"]["post"]["requestBody"]["content"] =
        serde_json_compat::json!({"multipart/form-data":{"schema":{"type":"object"}}});
    let source = super_source(&api.to_string());
    let inputs = [SynthesisSource {
        name: None,
        source: &source,
    }];
    let result = facade
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(result.coverage().exhaustive());
    assert!(!result.coverage().strict_eligible());
    assert!(
        result
            .coverage()
            .entries()
            .iter()
            .any(|e| e.status == CoverageStatus::ImplementationUnsupported)
    );
    assert!(
        result
            .coverage()
            .entries()
            .iter()
            .any(|e| e.status == CoverageStatus::Represented)
    );
    assert!(
        result
            .coverage()
            .entries()
            .iter()
            .all(|e| e.status != CoverageStatus::Excluded)
    );
}
fn super_source(text: &str) -> Source {
    source(text)
}
#[tokio::test]
async fn fresh_nested_resources_preserve_mounts_and_reference_bases() {
    let input = source(include_str!("fixtures/r1/nested-entry.json"));
    let mut options = Options::default();
    options.resources = vec![
        ResourceInput::at(
            "https://descriptions.example.invalid/fragments/parts.json",
            include_bytes!("fixtures/r1/nested-parts.json").as_slice(),
        )
        .unwrap(),
        ResourceInput::at(
            "https://descriptions.example.invalid/fragments/parameters.json",
            include_bytes!("fixtures/r1/nested-parameters.json").as_slice(),
        )
        .unwrap(),
    ];
    let provider = OpenApi::new(options).unwrap();
    let inspection = Inspector::new(&provider).inspect(&input).await.unwrap();
    assert!(inspection.exhaustive);
    assert_eq!(inspection.targets.len(), 2);
    let sources = [SynthesisSource {
        name: None,
        source: &input,
    }];
    let result = Synthesizer::new(&provider)
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &sources,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(result.coverage().strict_eligible());
    let draft = Document::from_json(result.document().parsed().value()).unwrap();
    let paths: std::collections::BTreeSet<_> = draft
        .bindings
        .unwrap()
        .into_values()
        .map(|b| {
            b.content
                .unwrap()
                .get("path")
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        paths,
        std::collections::BTreeSet::from([
            "/mounted/{parcelId}".into(),
            "/alternate/{parcelId}".into()
        ])
    );
}
#[tokio::test]
async fn exact_kind_patch_admission_and_carrier_boundaries() {
    let provider = OpenApi::default();
    let facade = Inspector::new(provider);
    for patch in ["3.1.0", "3.1.1", "3.1.2", "3.1.999"] {
        let text = include_str!("fixtures/r1/parcels.json").replace("3.1.2", patch);
        assert_eq!(
            facade.inspect(&source(&text)).await.unwrap().targets.len(),
            3
        );
    }
    for text in [
        "null",
        "{}",
        r#"{"document":{}}"#,
        r#"{"document":null}"#,
        r#"{"document":"{}","document":"{}"}"#,
        r#"{"document":"{}","surprise":true}"#,
    ] {
        let mut bad = source("{}");
        bad.content = Some(JsonValue::parse(text).unwrap());
        assert!(facade.inspect(&bad).await.is_err());
    }
    let mut bad = source(include_str!("fixtures/r1/parcels.json"));
    bad.kind = "some-other-kind".into();
    assert!(facade.inspect(&bad).await.is_err());
    bad.kind = EXPERIMENTAL_KIND.into();
    bad.content = None;
    assert!(facade.inspect(&bad).await.is_err());
}

#[tokio::test]
async fn inherited_metadata_is_charged_and_source_limit_has_an_exact_boundary() {
    let url = format!("https://example.invalid/{}", "x".repeat(8192));
    let paths: serde_json_compat::Map<_, _> = (0..8)
        .map(|n| {
            (
                format!("/n{n}"),
                serde_json_compat::json!({"get":{"responses":{"204":{"description":"empty"}}}}),
            )
        })
        .collect();
    let input=source(&serde_json_compat::json!({"openapi":"3.1.2","info":{"title":"Budget","version":"1"},"servers":[{"url":url}],"paths":paths}).to_string());
    let mut options = Options::default();
    options.result_bytes = 32768;
    let failure = Inspector::new(OpenApi::new(options).unwrap())
        .inspect(&input)
        .await
        .unwrap_err();
    assert_eq!(failure.code(), "openbindings.openapi.result-limit");
    let bytes = input.content.as_ref().unwrap().bytes().len();
    for (limit, accepted) in [(bytes, true), (bytes - 1, false)] {
        let mut options = Options::default();
        options.source_bytes = limit;
        assert_eq!(
            Inspector::new(OpenApi::new(options).unwrap())
                .inspect(&input)
                .await
                .is_ok(),
            accepted
        );
    }
}

#[tokio::test]
async fn response_profiles_and_ambiguous_mounts_have_truthful_coverage() {
    async fn coverage(api: serde_json_compat::Value) -> Vec<CoverageStatus> {
        let input = source(&api.to_string());
        let sources = [SynthesisSource {
            name: None,
            source: &input,
        }];
        Synthesizer::new(OpenApi::default())
            .synthesize_with_coverage(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap()
            .coverage()
            .entries()
            .iter()
            .map(|e| e.status)
            .collect()
    }
    let base = serde_json_compat::json!({"openapi":"3.1.2","info":{"title":"Profiles","version":"1"},"servers":[{"url":"https://example.invalid"}],"paths":{"/x":{"get":{"responses":{"204":{"description":"empty"}}}}}});
    let mut empty = base.clone();
    empty["paths"]["/x"]["get"]["responses"] =
        serde_json_compat::json!({"201":{"description":"created"}});
    assert_eq!(coverage(empty).await, vec![CoverageStatus::Represented]);
    let mut errors = base.clone();
    errors["paths"]["/x"]["get"]["responses"]["404"] = serde_json_compat::json!({"description":"problem","content":{"application/problem+json":{}}});
    let statuses = coverage(errors).await;
    assert!(statuses.contains(&CoverageStatus::Represented));
    assert!(statuses.contains(&CoverageStatus::Lossy));
    let mut ambiguous = base.clone();
    ambiguous["components"] =
        serde_json_compat::json!({"pathItems":{"Shared":base["paths"]["/x"].clone()}});
    ambiguous["paths"]["/x"]["$ref"] = serde_json_compat::json!("#/components/pathItems/Shared");
    assert!(
        coverage(ambiguous)
            .await
            .contains(&CoverageStatus::ImplementationUnsupported)
    );
    let mut unicode = base;
    let route = unicode["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/x")
        .unwrap();
    unicode["paths"]["/雪~s"] = route;
    assert_eq!(
        coverage(unicode).await,
        vec![CoverageStatus::ImplementationUnsupported]
    );
}

#[tokio::test]
async fn absent_and_repeated_operation_ids_are_not_binding_identity_or_effects() {
    use std::{cell::Cell, rc::Rc};
    let mut api: serde_json_compat::Value =
        serde_json_compat::from_str(include_str!("fixtures/r1/parcels.json")).unwrap();
    for path in api["paths"].as_object_mut().unwrap().values_mut() {
        for method in ["get", "post"] {
            if let Some(op) = path.get_mut(method) {
                op["operationId"] = serde_json_compat::json!("same");
            }
        }
    }
    let input = source(&api.to_string());
    let entries = Rc::new(Cell::new(0));
    let recorded = entries.clone();
    let provider = OpenApi::default().with_transport(
        move |_: dynamic_openapi_client::transport::PreparedRequest,
              _: dynamic_openapi_client::Cancellation| {
            recorded.set(recorded.get() + 1);
            std::future::pending::<dynamic_openapi_client::Outcome>()
        },
    );
    let inspection = Inspector::new(&provider).inspect(&input).await.unwrap();
    assert_eq!(inspection.targets.len(), 3);
    let inputs = [SynthesisSource {
        name: None,
        source: &input,
    }];
    let result = Synthesizer::new(&provider)
        .synthesize(&SynthesisRequest {
            sources: &inputs,
            ..Default::default()
        })
        .await
        .unwrap();
    let draft = Document::from_json(result.parsed().value()).unwrap();
    assert_eq!(draft.operations.len(), 3);
    assert_eq!(draft.bindings.unwrap().len(), 3);
    assert_eq!(entries.get(), 0);
    // This is robustness evidence only. Duplicate operationId violates upstream
    // uniqueness; neither inspection nor synthesis asserts source conformance.
}

#[tokio::test]
async fn synthesized_contracts_keep_verdicts_with_explicit_diagnostics() {
    use openbindings::{Direction, ResourceSet, ValueContractPreparation, ValueOutcome};
    use openbindings_json_schema_evaluator::{DefaultEvaluator, Diagnostics};
    use std::sync::Arc;

    // Source schema references are disclosed as projection loss, never copied
    // into the normative envelope contract, even when statically defective.
    for version in ["3.0.4", "3.1.2"] {
        let api = serde_json_compat::json!({
            "openapi": version,
            "servers": [{"url": "https://example.invalid"}],
            "paths": {"/items": {"post": {
                "requestBody": {"required": true, "content": {"application/json": {
                    "schema": {"$ref": "#/missing"}
                }}},
                "responses": {"204": {"description": "empty"}}
            }}}
        });
        let mut input = source(&api.to_string());
        if version == "3.0.4" {
            input.kind = openbindings_openapi::EXPERIMENTAL_KIND_30.into();
        }
        let inputs = [SynthesisSource {
            name: None,
            source: &input,
        }];
        let result = Synthesizer::new(OpenApi::default())
            .synthesize_with_coverage(&SynthesisRequest {
                sources: &inputs,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            result
                .coverage()
                .entries()
                .iter()
                .any(|e| e.status == CoverageStatus::Lossy)
        );
        let proof = result.document();
        let operations = proof.operations();
        let operation = &operations[0];
        assert!(operation.input().is_some());
        for mode in [Diagnostics::Coarse, Diagnostics::Upstream] {
            let context = proof.value_contracts(
                Arc::new(DefaultEvaluator::new().with_diagnostics(mode)),
                ResourceSet::default(),
            );
            let ValueContractPreparation::Ready(contract) =
                context.prepare(operation.key(), Direction::Input)
            else {
                panic!("synthesized envelope must prepare");
            };
            assert!(matches!(
                contract.validate(&JsonValue::parse(r#"{"body":null}"#).unwrap()),
                ValueOutcome::Satisfies
            ));
            let ValueOutcome::Fails {
                message,
                problems,
                problems_complete,
            } = contract.validate(&JsonValue::parse("{}").unwrap())
            else {
                panic!("missing body must fail");
            };
            assert_eq!(message, "value does not satisfy the contract");
            assert!(!problems_complete);
            match mode {
                Diagnostics::Coarse => assert!(problems.is_empty()),
                Diagnostics::Upstream => assert_eq!(problems.len(), 1),
            }
        }
    }
}
