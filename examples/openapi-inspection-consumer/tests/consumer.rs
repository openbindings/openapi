use dynamic_openapi_client::{Document as ClientDocument, Limits, inspect::Method};
use openapi_inspection_consumer::{author, inspect_and_author, source};
use openbindings::{Binding, Document, JsonKind, JsonValue};

const LOCATION: &str = "https://fixture.invalid/openapi.json";

/// Source content carrying the description as JSON text, a string `document`.
fn carrier(document: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "document": document,
        "location": LOCATION
    }))
    .unwrap()
}

/// Source content embedding the description as a JSON object, its text spliced in as
/// written so its numbers keep their exact values.
fn embedded(document: &str) -> String {
    format!(r#"{{"document":{document},"location":"{LOCATION}"}}"#)
}

#[tokio::test]
async fn public_inspection_manual_authoring_and_exact_reload() {
    let description = r#"{"openapi":"3.1.2","x-exact":9007199254740993,"components":{"schemas":{"Opaque":{"$schema":"urn:example:unknown-dialect","const":9007199254740993}}},"paths":{"/a~b/c":{"get":{"operationId":"totally.different-É"}}}}"#;
    for (content, kind) in [
        (carrier(description), JsonKind::String),
        (embedded(description), JsonKind::Object),
    ] {
        authored_and_reloaded(&content, kind).await;
    }
}

async fn authored_and_reloaded(content: &str, kind: JsonKind) {
    let result = inspect_and_author(content, "/paths/~1a~0b~1c/get")
        .await
        .unwrap();
    assert!(result.text().contains("9007199254740993"));
    assert!(result.text().contains("urn:example:unknown-dialect"));
    let reloaded = Document::from_json(result.clone());
    let source = &reloaded.source("catalog-source").unwrap().unwrap();
    // The OBI carries the source as authored: an embedded object stays an object, and its
    // own text is the description the client reads.
    let document = source.content().unwrap().get("document").unwrap();
    assert_eq!(document.kind(), kind);
    let client = ClientDocument::parse(
        document.as_str().unwrap_or(document.text()),
        Some(LOCATION),
        Limits::default(),
    )
    .unwrap();
    let operation = client.operation("/a~b/c", Method::Get).unwrap();
    assert_eq!(operation.id().as_deref(), Some("totally.different-É"));
    assert_eq!(
        client.root().get("x-exact").unwrap().raw(),
        "9007199254740993"
    );
    let binding = &reloaded.binding("catalog-binding").unwrap().unwrap();
    assert_eq!(
        binding.content().unwrap().get("path").unwrap().as_str(),
        Some("/a~b/c")
    );
    assert_eq!(
        binding.content().unwrap().get("method").unwrap().as_str(),
        Some("get")
    );
    // Independent manual OBI construction does not use inspection suggestions.
    let manual = author(
        openapi_inspection_consumer::source(content).unwrap(),
        Binding {
            operation: "catalog.lookup".into(),
            source: "catalog-source".into(),
            content: Some(JsonValue::parse(r#"{"path":"/a~b/c","method":"get"}"#).unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    let manual_reload = Document::from_json(manual.clone());
    let manual_view = manual_reload.binding("catalog-binding").unwrap().unwrap();
    let manual_binding = manual_view.content().unwrap();
    assert_eq!(manual_binding.get("path").unwrap().as_str(), Some("/a~b/c"));
    assert_eq!(manual_binding.get("method").unwrap().as_str(), Some("get"));
    assert!(
        Document::from_json(manual.clone())
            .validate()
            .unwrap()
            .conformant()
            .is_some()
    );
}
#[tokio::test]
async fn real_resource_recovery_from_public_packages() {
    use openbindings_openapi::{OpenApi, Options, ResourceInput};
    use openbindings_source_inspection::Inspector;
    let source = source(&carrier(r#"{"openapi":"3.1.2","paths":{"/mounted":{"$ref":"./paths.json#/components/pathItems/P"}}}"#)).unwrap();
    let partial = Inspector::new(OpenApi::default())
        .inspect(&source)
        .await
        .unwrap();
    assert!(!partial.exhaustive);
    let mut options = Options::default();
    options.resources.push(
        ResourceInput::at(
            "https://fixture.invalid/paths.json",
            &br#"{"openapi":"3.1.2","components":{"pathItems":{"P":{"get":{}}}}}"#[..],
        )
        .unwrap(),
    );
    let full = Inspector::new(OpenApi::new(options).unwrap())
        .inspect(&source)
        .await
        .unwrap();
    assert!(full.exhaustive);
    assert_eq!(full.targets[0].source_ref, "/paths/~1mounted/get");
    let authored = openapi_inspection_consumer::author_selected(
        source,
        full.targets.into_iter().next().unwrap(),
    )
    .unwrap();
    let reloaded = Document::from_json(authored.clone());
    let source = &reloaded.source("catalog-source").unwrap().unwrap();
    let content = source.content().unwrap();
    let mut graph = dynamic_openapi_client::graph::DocumentBuilder::new(
        Limits::default(),
        dynamic_openapi_client::graph::GraphLimits::default(),
    );
    let entry = graph
        .add(
            ResourceInput::at(
                content.get("location").unwrap().as_str().unwrap(),
                content
                    .get("document")
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap(),
        )
        .unwrap();
    graph
        .add(
            ResourceInput::at(
                "https://fixture.invalid/paths.json",
                &br#"{"openapi":"3.1.2","components":{"pathItems":{"P":{"get":{}}}}}"#[..],
            )
            .unwrap(),
        )
        .unwrap();
    let document = graph.build(entry).unwrap();
    let operation = document.operation("/mounted", Method::Get).unwrap();
    assert_eq!(operation.path(), "/mounted");
    assert_eq!(operation.value().raw(), "{}");
    assert_eq!(
        reloaded
            .binding("catalog-binding")
            .unwrap()
            .unwrap()
            .content()
            .unwrap()
            .get("path")
            .unwrap()
            .as_str(),
        Some("/mounted")
    );
}

#[tokio::test]
async fn inspected_neighbors_remain_preparable_after_manual_authoring_and_reload() {
    use dynamic_openapi_client::diagnostics::Code;
    use openbindings_openapi::OpenApi;
    use openbindings_source_inspection::Inspector;
    for (path_item, target, method, expected_targets) in [
        (
            r#"{"get":{"operationId":"read"},"future":true}"#,
            "/paths/~1x/get",
            Method::Get,
            1,
        ),
        (
            r##"{"$ref":"#/components/pathItems/P","get":{"operationId":"outer"}}"##,
            "/paths/~1x/post",
            Method::Post,
            2,
        ),
    ] {
        let document = format!(
            r#"{{"openapi":"3.1.2","servers":[{{"url":"https://api.example"}}],"paths":{{"/x":{path_item}}},"components":{{"pathItems":{{"P":{{"get":{{"operationId":"inner"}},"post":{{"operationId":"create"}}}}}}}}}}"#
        );
        let content = carrier(&document);
        let inventory = Inspector::new(OpenApi::default())
            .inspect(&source(&content).unwrap())
            .await
            .unwrap();
        assert!(inventory.exhaustive);
        assert_eq!(inventory.targets.len(), expected_targets);
        assert!(
            inventory
                .targets
                .iter()
                .any(|entry| entry.source_ref == target)
        );
        // Inspection includes known unsupported targets; authoring selects
        // the neighbor whose direct client preparation succeeds.
        let authored = inspect_and_author(&content, target).await.unwrap();
        let reloaded = Document::from_json(authored.clone());
        let source_view = reloaded.source("catalog-source").unwrap().unwrap();
        let document = source_view.content().unwrap().get("document").unwrap();
        let client =
            ClientDocument::parse(document.as_str().unwrap(), None, Limits::default()).unwrap();
        let operation = client.operation("/x", method.clone()).unwrap();
        let request = operation.request().prepare().unwrap();
        assert_eq!(request.method(), method);
        assert_eq!(request.target(), "https://api.example/x");
        if method == Method::Post {
            assert_eq!(
                client
                    .operation("/x", Method::Get)
                    .unwrap()
                    .request()
                    .prepare()
                    .unwrap_err()
                    .code(),
                Code::AmbiguousReference
            );
        }
    }
}
