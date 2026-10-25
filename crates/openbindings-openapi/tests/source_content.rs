//! Source content: `document` embedded as an object or as JSON text, `location` as its
//! identity and reference base, and the forms this implementation refuses.
use dynamic_openapi_client::{
    Cancellation, Header, Outcome,
    transport::{
        BodyState, HeaderDelivery, HostCapabilities, PreparedRequest, Provenance, ResponseInput,
        TransportResult, UploadState,
    },
};
use dynamic_openapi_client::{graph::ReferenceKind, inspect::Edition};
use openbindings::{Document, JsonValue, Source};
use openbindings_interface_synthesis::{
    SynthesisErrorKind, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_invocation::{BindingRequest, BindingValue, binding, operation};
use openbindings_openapi::{KIND_30, KIND_31, OpenApi, Options, ResourceInput, ResourceLayout};
use openbindings_source_inspection::{InspectionErrorKind, Inspector};
use std::{cell::RefCell, rc::Rc, sync::Arc};

const LOCATION: &str = "https://descriptions.example.invalid/catalog/root.json";
/// An entry document whose spacing and number spellings a re-serialization would change.
fn embedded(version: &str) -> String {
    format!(
        r#"{{ "openapi" : "{version}",
  "info": {{"title": "Embedded", "version": "1"}},
  "x-exact": 9007199254740993, "x-ratio": 1.50, "x-tiny": 1E-400,
  "servers": [{{"url": "https://embedded.example.test"}}],
  "paths": {{
    "/items/{{id}}": {{"get": {{
      "parameters": [{{"name": "id", "in": "path", "required": true, "schema": {{"type": "string"}}}}],
      "responses": {{"200": {{"description": "JSON", "content": {{"application/json": {{}}}}}}}}
    }}}},
    "/mounted": {{"$ref": "./parts.json#/Mounted"}}
  }}
}}"#
    )
}
/// A supplied fragment bundle holding the mounted Path Item, beside the entry document.
fn options(version: &str) -> Options {
    let edition = if version.starts_with("3.0") {
        Edition::OpenApi30
    } else {
        Edition::OpenApi31
    };
    let mut options = Options::default();
    options.resources = vec![
        ResourceInput::at(
            "https://descriptions.example.invalid/catalog/parts.json",
            &br#"{"Mounted":{"get":{"responses":{"204":{"description":"empty"}}}}}"#[..],
        )
        .unwrap()
        .layout(ResourceLayout::new().protocol(
            "/Mounted",
            ReferenceKind::PathItem,
            edition,
        )),
    ];
    options
}
fn source(kind: &str, content: &str) -> Source {
    Source {
        kind: kind.into(),
        content: Some(JsonValue::parse(content).unwrap()),
        ..Default::default()
    }
}
/// Records the exact entry-document text the client analyzed, then answers 200 `null`.
type Seen = Rc<RefCell<Vec<(String, String)>>>;
fn host(
    seen: Seen,
) -> impl Fn(PreparedRequest, Cancellation) -> std::pin::Pin<Box<dyn Future<Output = Outcome>>> + Clone
{
    move |request, cancellation| {
        let seen = seen.clone();
        Box::pin(async move {
            seen.borrow_mut().push((
                request.target().to_owned(),
                request.operation().value().source().to_owned(),
            ));
            request
                .invoke(
                    cancellation,
                    &HostCapabilities::programmable(),
                    |_, _| async {
                        let mut response = ResponseInput::new(
                            200,
                            vec![Header::new("content-type", "application/json")],
                            Arc::from(&b"null"[..]),
                            BodyState::Complete,
                            Provenance::Wire,
                        );
                        response.header_delivery = HeaderDelivery::Complete;
                        TransportResult {
                            upload: UploadState::Complete,
                            response: Some(response),
                            error_detail: None,
                        }
                    },
                )
                .await
        })
    }
}
async fn call<P: binding::BindingInvoker>(
    provider: P,
    source: &Source,
    path: &str,
    input: &str,
) -> Result<Vec<String>, openbindings_invocation::InvocationError> {
    let binding = BindingValue {
        content: Some(
            JsonValue::parse(serde_json_compat::json!({"path":path,"method":"get"}).to_string())
                .unwrap(),
        ),
        ..Default::default()
    };
    let invoker = binding::Invoker::new(provider);
    let mut job = invoker.invoke(BindingRequest {
        source,
        binding: &binding,
        context: None,
    });
    let _ = job.write(JsonValue::parse(input).unwrap()).await;
    job.close();
    let mut values = vec![];
    while let Some(value) = job.next().await {
        values.push(value?.text().to_owned());
    }
    job.outcome().expect("terminal")?;
    Ok(values)
}

#[tokio::test]
async fn an_embedded_document_reaches_the_client_as_its_exact_json_text() {
    for (kind, version) in [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")] {
        let document = embedded(version);
        let content = format!(r#"{{"document": {document}, "location": "{LOCATION}"}}"#);
        let src = source(kind, &content);
        // `location` is the base for the relative Path Item reference.
        let provider = OpenApi::new(options(version)).unwrap();
        let inventory = Inspector::new(&provider).inspect(&src).await.unwrap();
        assert!(inventory.exhaustive);
        assert_eq!(inventory.targets.len(), 2);
        let sources = [SynthesisSource {
            name: Some("catalog"),
            source: &src,
        }];
        let synthesized = Synthesizer::new(&provider)
            .synthesize(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap();
        // Synthesis carries the supplied content unchanged, embedded object included.
        let saved = synthesized.as_document().value().text().to_owned();
        let reloaded = Document::parse(&saved).unwrap();
        assert_eq!(
            reloaded
                .source("catalog")
                .unwrap()
                .unwrap()
                .content()
                .unwrap()
                .text(),
            src.content.as_ref().unwrap().text()
        );
        let key = reloaded
            .bindings()
            .unwrap()
            .unwrap()
            .into_iter()
            .find(|b| b.content().unwrap().get("path").unwrap().as_str() == Some("/items/{id}"))
            .unwrap()
            .operation()
            .unwrap()
            .to_owned();
        let seen = Seen::default();
        let invoker = operation::Invoker::new(
            reloaded.validate().unwrap().conformant().unwrap(),
            OpenApi::new(options(version))
                .unwrap()
                .with_transport(host(seen.clone())),
        )
        .with_evaluator(
            Arc::new(openbindings_json_schema_evaluator::DefaultEvaluator::new()),
            openbindings::ResourceSet::default(),
        );
        let mut job = invoker.invoke(operation::Request::operation(&key));
        let _ = job
            .write(JsonValue::parse(r#"{"path":{"id":"box-7"}}"#).unwrap())
            .await;
        job.close();
        assert_eq!(job.next().await.unwrap().unwrap().text(), "null");
        assert!(job.next().await.is_none());
        job.outcome().unwrap().unwrap();
        let seen = seen.borrow();
        assert_eq!(seen[0].0, "https://embedded.example.test/items/box-7");
        // Exactly the member's text: spacing, `9007199254740993`, `1.50` and `1E-400`.
        assert_eq!(seen[0].1, document);
    }
}

#[tokio::test]
async fn document_text_and_an_embedded_object_describe_the_same_operations() {
    for (kind, version) in [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")] {
        let document = embedded(version);
        let as_text = serde_json_compat::json!({"document": document, "location": LOCATION});
        let text_source = source(kind, &as_text.to_string());
        let object_source = source(
            kind,
            &format!(r#"{{"location":"{LOCATION}","document":{document}}}"#),
        );
        let provider = OpenApi::new(options(version)).unwrap();
        let inspect = |src: Source| {
            let provider = &provider;
            async move {
                Inspector::new(provider)
                    .inspect(&src)
                    .await
                    .unwrap()
                    .to_json()
                    .unwrap()
                    .text()
                    .to_owned()
            }
        };
        assert_eq!(
            inspect(text_source.clone()).await,
            inspect(object_source.clone()).await
        );
        for src in [&text_source, &object_source] {
            let seen = Seen::default();
            let provider = OpenApi::new(options(version))
                .unwrap()
                .with_transport(host(seen.clone()));
            assert_eq!(
                call(&provider, src, "/items/{id}", r#"{"path":{"id":"a"}}"#)
                    .await
                    .unwrap(),
                ["null"]
            );
            assert_eq!(
                call(&provider, src, "/mounted", "{}").await.unwrap(),
                ["null"]
            );
            assert_eq!(seen.borrow()[0].1, document);
        }
    }
}

#[tokio::test]
async fn yaml_document_text_is_refused_as_unsupported() {
    let yaml = include_str!("fixtures/minimal-31.yaml");
    for (kind, text) in [
        (KIND_31, yaml.to_owned()),
        (KIND_30, yaml.replace("3.1.2", "3.0.4")),
    ] {
        let src = source(
            kind,
            &serde_json_compat::json!({"document": text}).to_string(),
        );
        let inspection = Inspector::new(OpenApi::default())
            .inspect(&src)
            .await
            .unwrap_err();
        assert_eq!(inspection.kind(), InspectionErrorKind::UnreadableSource);
        assert_eq!(
            inspection.code(),
            "openbindings.openapi.yaml-document-unimplemented"
        );
        let sources = [SynthesisSource {
            name: None,
            source: &src,
        }];
        let synthesis = Synthesizer::new(OpenApi::default())
            .synthesize(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(synthesis.kind(), SynthesisErrorKind::UnsupportedRequest);
        assert_eq!(
            synthesis.code(),
            "openbindings.openapi.yaml-document-unimplemented"
        );
        let seen = Seen::default();
        let provider = OpenApi::default().with_transport(host(seen.clone()));
        assert_eq!(
            call(&provider, &src, "/items", "{}")
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        assert!(seen.borrow().is_empty());
    }
}

#[tokio::test]
async fn a_location_without_a_document_is_never_fetched() {
    for (kind, version) in [(KIND_31, "3.1.2"), (KIND_30, "3.0.4")] {
        let src = source(kind, &format!(r#"{{"location":"{LOCATION}"}}"#));
        // Even bytes supplied at that very identity do not stand in for `document`.
        let mut supplied = options(version);
        supplied
            .resources
            .push(ResourceInput::at(LOCATION, embedded(version).into_bytes()).unwrap());
        let inspection = Inspector::new(OpenApi::new(supplied.clone()).unwrap())
            .inspect(&src)
            .await
            .unwrap_err();
        assert_eq!(inspection.kind(), InspectionErrorKind::UnreadableSource);
        assert_eq!(
            inspection.code(),
            "openbindings.openapi.document-retrieval-unimplemented"
        );
        let sources = [SynthesisSource {
            name: None,
            source: &src,
        }];
        let synthesis = Synthesizer::new(OpenApi::new(supplied.clone()).unwrap())
            .synthesize(&SynthesisRequest {
                sources: &sources,
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(synthesis.kind(), SynthesisErrorKind::UnsupportedRequest);
        assert_eq!(
            synthesis.code(),
            "openbindings.openapi.document-retrieval-unimplemented"
        );
        let seen = Seen::default();
        let provider = OpenApi::new(supplied)
            .unwrap()
            .with_transport(host(seen.clone()));
        assert_eq!(
            call(&provider, &src, "/items/{id}", r#"{"path":{"id":"a"}}"#)
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        assert!(seen.borrow().is_empty());
    }
}

#[tokio::test]
async fn content_members_and_forms_outside_the_kind_are_refused() {
    let text = serde_json_compat::to_string(&embedded("3.1.2")).unwrap();
    let object = embedded("3.1.2");
    for (content, kind, code) in [
        (
            "null".to_owned(),
            InspectionErrorKind::InvalidInput,
            "expected-object",
        ),
        (
            r#""text""#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "expected-object",
        ),
        (
            "{}".to_owned(),
            InspectionErrorKind::InvalidInput,
            "document-required",
        ),
        (
            r#"{"document":7}"#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "document-invalid",
        ),
        (
            r#"{"document":null}"#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "document-invalid",
        ),
        (
            r#"{"document":[]}"#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "document-invalid",
        ),
        (
            r#"{"document":"\ud800"}"#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "document-invalid",
        ),
        // The earlier carrier's identity member has no meaning under these kinds.
        (
            format!(r#"{{"document":{text},"retrievalUri":"{LOCATION}"}}"#),
            InspectionErrorKind::InvalidInput,
            "unknown-or-duplicate-member",
        ),
        (
            format!(r#"{{"document":{object},"surprise":true}}"#),
            InspectionErrorKind::InvalidInput,
            "unknown-or-duplicate-member",
        ),
        (
            format!(r#"{{"document":{text},"document":{text}}}"#),
            InspectionErrorKind::InvalidInput,
            "unknown-or-duplicate-member",
        ),
        (
            format!(r#"{{"document":{object},"location":17}}"#),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        (
            r#"{"location":17}"#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        // `location` is an absolute URI: a relative reference, or one with a fragment,
        // is not one, whether or not `document` is present.
        (
            format!(r#"{{"document":{object},"location":"catalog/root.json"}}"#),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        (
            format!(r#"{{"document":{object},"location":"/catalog/root.json"}}"#),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        (
            format!(
                r#"{{"document":{object},"location":"//descriptions.example.invalid/root.json"}}"#
            ),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        (
            format!(r#"{{"document":{object},"location":"{LOCATION}#root"}}"#),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        (
            format!(r#"{{"document":{object},"location":"https://bad host/root.json"}}"#),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        (
            r#"{"location":"catalog/root.json"}"#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "location-invalid",
        ),
        // An absolute URI the client cannot use as a retrieval identity is unsupported.
        (
            format!(
                r#"{{"document":{object},"location":"https://user@descriptions.example.invalid/root.json"}}"#
            ),
            InspectionErrorKind::UnreadableSource,
            "location-unsupported",
        ),
        // An embedded document is read as JSON, so its duplicate members stay unusable.
        (
            r#"{"document":{"openapi":"3.1.2","openapi":"3.1.2","paths":{}}}"#.to_owned(),
            InspectionErrorKind::InvalidInput,
            "document-admission",
        ),
    ] {
        let src = source(KIND_31, &content);
        let error = Inspector::new(OpenApi::new(options("3.1.2")).unwrap())
            .inspect(&src)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), kind, "{content}");
        assert_eq!(
            error.code(),
            format!("openbindings.openapi.{code}"),
            "{content}"
        );
        let seen = Seen::default();
        let provider = OpenApi::new(options("3.1.2"))
            .unwrap()
            .with_transport(host(seen.clone()));
        assert_eq!(
            call(&provider, &src, "/items/{id}", r#"{"path":{"id":"a"}}"#)
                .await
                .unwrap_err()
                .code(),
            "ERR_REFUSED"
        );
        assert!(seen.borrow().is_empty());
    }
}
