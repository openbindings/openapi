//! Generated operation keys and carried Operation Object metadata.
//!
//! Every case runs on both editions. Synthesis and inspection must agree: each
//! synthesized operation's key is inspection's `operationKey` suggestion for the
//! same target, and its operation is inspection's operation framing.
use openbindings::{JsonValue, Source};
use openbindings_interface_synthesis::{
    CoverageStatus, SynthesisRequest, SynthesisSource, Synthesizer,
};
use openbindings_openapi::{KIND_30, KIND_31, OpenApi};
use openbindings_source_inspection::Inspector;
use serde_json_compat::{Value, json};
use std::collections::BTreeMap;

const EDITIONS: [(&str, &str); 2] = [(KIND_30, "3.0.4"), (KIND_31, "3.1.2")];

/// A supported Operation Object: no parameters and an empty success, plus `fields`.
fn op(fields: Value) -> Value {
    let mut operation = json!({"responses":{"204":{"description":"empty"}}});
    let members = operation.as_object_mut().unwrap();
    for (name, value) in fields.as_object().unwrap() {
        members.insert(name.clone(), value.clone());
    }
    operation
}

/// Description text with Path Items in exactly the given order.
fn document(version: &str, paths: &[(&str, Value)]) -> String {
    let paths: Vec<_> = paths
        .iter()
        .map(|(path, item)| format!("{}:{item}", json!(path)))
        .collect();
    format!(
        r#"{{"openapi":"{version}","info":{{"title":"Names","version":"1"}},"servers":[{{"url":"https://names.example.invalid"}}],"paths":{{{}}}}}"#,
        paths.join(",")
    )
}

/// `"{method} {path}"` for a `/paths/...` source reference.
fn target_name(source_ref: &str) -> String {
    let (path, method) = source_ref
        .strip_prefix("/paths/")
        .unwrap()
        .rsplit_once('/')
        .unwrap();
    format!("{method} {}", path.replace("~1", "/").replace("~0", "~"))
}

struct Generated {
    /// Synthesized operation key and operation by `"{method} {path}"`.
    operations: BTreeMap<String, (String, Value)>,
    /// Inspection's key suggestion for every target by `"{method} {path}"`.
    suggestions: BTreeMap<String, Option<String>>,
    /// Coverage reason code of each implementation-unsupported target.
    unsupported: BTreeMap<String, String>,
}
impl Generated {
    fn keys(&self) -> BTreeMap<&str, &str> {
        self.operations
            .iter()
            .map(|(name, (key, _))| (name.as_str(), key.as_str()))
            .collect()
    }
    fn suggested(&self) -> BTreeMap<&str, Option<&str>> {
        self.suggestions
            .iter()
            .map(|(name, key)| (name.as_str(), key.as_deref()))
            .collect()
    }
    /// The synthesized operation's members other than its generated input contract.
    fn metadata(&self, name: &str) -> Value {
        let mut operation = self.operations[name].1.clone();
        assert!(operation.as_object_mut().unwrap().remove("input").is_some());
        operation
    }
}

async fn generate(kind: &str, text: &str) -> Generated {
    let source = Source {
        kind: kind.into(),
        content: Some(JsonValue::parse(json!({ "document": text }).to_string()).unwrap()),
        ..Default::default()
    };
    let provider = OpenApi::default();
    let inspection = Inspector::new(&provider).inspect(&source).await.unwrap();
    let wire: Value = serde_json_compat::from_str(inspection.to_json().unwrap().text()).unwrap();
    let mut suggestions = BTreeMap::new();
    let mut framings = BTreeMap::new();
    for target in wire["targets"].as_array().unwrap() {
        let name = target_name(target["sourceRef"].as_str().unwrap());
        let content = &target["binding"]["content"];
        assert_eq!(
            name,
            format!(
                "{} {}",
                content["method"].as_str().unwrap(),
                content["path"].as_str().unwrap()
            )
        );
        suggestions.insert(
            name.clone(),
            target
                .get("operationKey")
                .map(|key| key.as_str().unwrap().to_owned()),
        );
        framings.insert(name, target.get("operation").cloned());
    }
    let sources = [SynthesisSource {
        name: None,
        source: &source,
    }];
    let result = Synthesizer::new(&provider)
        .synthesize_with_coverage(&SynthesisRequest {
            sources: &sources,
            ..Default::default()
        })
        .await
        .unwrap();
    let draft = result.document();
    let declared: BTreeMap<_, _> = draft
        .operations()
        .iter()
        .map(|o| {
            let value: Value = serde_json_compat::from_str(o.value().text()).unwrap();
            (o.key().to_owned(), value)
        })
        .collect();
    let bindings = draft.bindings().unwrap_or_default();
    assert_eq!(bindings.len(), declared.len());
    let mut operations = BTreeMap::new();
    for binding in &bindings {
        // The binding key equals the operation key, and binding content is unchanged.
        assert_eq!(binding.operation(), binding.key());
        let content: Value =
            serde_json_compat::from_str(binding.content().unwrap().text()).unwrap();
        assert_eq!(content.as_object().unwrap().len(), 2);
        let name = format!(
            "{} {}",
            content["method"].as_str().unwrap(),
            content["path"].as_str().unwrap()
        );
        let operation = declared[binding.key()].clone();
        assert_eq!(suggestions[&name].as_deref(), Some(binding.key()), "{name}");
        assert_eq!(framings[&name].as_ref(), Some(&operation), "{name}");
        operations.insert(name, (binding.key().to_owned(), operation));
    }
    let mut unsupported = BTreeMap::new();
    for entry in result.coverage().entries() {
        if entry.status == CoverageStatus::ImplementationUnsupported {
            assert!(entry.operation_key.is_none());
            unsupported.insert(
                target_name(&entry.source_ref),
                entry.reason_code.clone().unwrap(),
            );
        }
    }
    Generated {
        operations,
        suggestions,
        unsupported,
    }
}

#[tokio::test]
async fn operation_ids_matching_the_key_grammar_are_kept_exactly() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                (
                    "/parcels",
                    json!({
                        "get": op(json!({"operationId": "Parcels.list-v2_0"})),
                        "post": op(json!({"operationId": "_create"}))
                    }),
                ),
                (
                    "/parcels/{parcelId}",
                    json!({"get": op(json!({"operationId": "9lives"}))}),
                ),
            ],
        );
        assert_eq!(
            generate(kind, &text).await.keys(),
            BTreeMap::from([
                ("get /parcels", "Parcels.list-v2_0"),
                ("post /parcels", "_create"),
                ("get /parcels/{parcelId}", "9lives"),
            ])
        );
    }
}

#[tokio::test]
async fn operation_ids_outside_the_key_grammar_are_sanitized_without_case_change() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                (
                    "/spaces",
                    json!({"get": op(json!({"operationId": "List Parcels/v2"}))}),
                ),
                (
                    "/dot",
                    json!({"get": op(json!({"operationId": ".hidden"}))}),
                ),
                (
                    "/dash",
                    json!({"get": op(json!({"operationId": "-Dashed"}))}),
                ),
                (
                    "/unicode",
                    json!({"get": op(json!({"operationId": "雪.Query"}))}),
                ),
                (
                    "/symbol",
                    json!({"get": op(json!({"operationId": "@Home"}))}),
                ),
            ],
        );
        assert_eq!(
            generate(kind, &text).await.keys(),
            BTreeMap::from([
                ("get /spaces", "List_Parcels_v2"),
                ("get /dot", "_.hidden"),
                ("get /dash", "_-Dashed"),
                ("get /unicode", "_.Query"),
                ("get /symbol", "_Home"),
            ])
        );
    }
}

#[tokio::test]
async fn empty_operation_ids_are_treated_as_absent() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                (
                    "/tasks/{id}/comments",
                    json!({"get": op(json!({"operationId": ""}))}),
                ),
                // Two empty identifiers are not a duplicated identifier.
                ("/tasks", json!({"post": op(json!({"operationId": ""}))})),
                (
                    "/labels",
                    json!({"get": op(json!({"operationId": "listLabels"}))}),
                ),
            ],
        );
        assert_eq!(
            generate(kind, &text).await.keys(),
            BTreeMap::from([
                ("get /tasks/{id}/comments", "get_tasks_id_comments"),
                ("post /tasks", "post_tasks"),
                ("get /labels", "listLabels"),
            ])
        );
    }
}

#[tokio::test]
async fn duplicated_operation_ids_fall_back_to_derived_keys() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                (
                    "/parcels",
                    json!({
                        "get": op(json!({"operationId": "same"})),
                        "post": op(json!({"operationId": "same"}))
                    }),
                ),
                (
                    "/parcels/{parcelId}",
                    json!({
                        // Exact comparison: a different case is a different identifier.
                        "delete": op(json!({"operationId": "Same"})),
                        "get": op(json!({"operationId": "unique"}))
                    }),
                ),
            ],
        );
        assert_eq!(
            generate(kind, &text).await.keys(),
            BTreeMap::from([
                ("get /parcels", "get_parcels"),
                ("post /parcels", "post_parcels"),
                ("delete /parcels/{parcelId}", "Same"),
                ("get /parcels/{parcelId}", "unique"),
            ])
        );
    }
}

#[tokio::test]
async fn derived_keys_use_the_method_and_normalized_path_segments() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                ("/", json!({"get": op(json!({}))})),
                ("/tasks/{id}/comments", json!({"get": op(json!({}))})),
                ("/files/{name}.json", json!({"delete": op(json!({}))})),
                ("/caf%C3%A9/{item-id}", json!({"put": op(json!({}))})),
                ("/a__b/_c_/", json!({"patch": op(json!({}))})),
            ],
        );
        assert_eq!(
            generate(kind, &text).await.keys(),
            BTreeMap::from([
                ("get /", "get"),
                ("get /tasks/{id}/comments", "get_tasks_id_comments"),
                ("delete /files/{name}.json", "delete_files_name.json"),
                ("put /caf%C3%A9/{item-id}", "put_caf_C3_A9_item-id"),
                ("patch /a__b/_c_/", "patch_a_b_c"),
            ])
        );
    }
}

// FNV-1a 64 of "get /a/b" is 18cf5906f8a263bc and of "get /a_b" is 17523906f75e284c;
// of "get /x" b0f6c71fe22e60c2 and of "post /y" 1b9eb28c328d1d45.
#[tokio::test]
async fn colliding_keys_take_a_method_and_path_hash_suffix() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                ("/a/b", json!({"get": op(json!({}))})),
                ("/a_b", json!({"get": op(json!({}))})),
                // `-` is a key character, so this path derives a distinct key.
                ("/a-b", json!({"get": op(json!({}))})),
                ("/x", json!({"get": op(json!({}))})),
                // An identifier equal to another operation's derived key collides too;
                // both then take their derived keys with a suffix.
                ("/y", json!({"post": op(json!({"operationId": "get_x"}))})),
            ],
        );
        assert_eq!(
            generate(kind, &text).await.keys(),
            BTreeMap::from([
                ("get /a/b", "get_a_b_18cf5906"),
                ("get /a_b", "get_a_b_17523906"),
                ("get /a-b", "get_a-b"),
                ("get /x", "get_x_b0f6c71f"),
                ("post /y", "post_y_1b9eb28c"),
            ])
        );
    }
}

#[tokio::test]
async fn suffixed_keys_that_still_collide_take_the_full_hash() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                ("/a/b", json!({"get": op(json!({}))})),
                ("/a_b", json!({"get": op(json!({}))})),
                (
                    "/r",
                    json!({"get": op(json!({"operationId": "get_a_b_18cf5906"}))}),
                ),
            ],
        );
        assert_eq!(
            generate(kind, &text).await.keys(),
            BTreeMap::from([
                ("get /a/b", "get_a_b_18cf5906f8a263bc"),
                ("get /a_b", "get_a_b_17523906"),
                ("get /r", "get_a_b_18cf5906"),
            ])
        );
    }
}

#[tokio::test]
async fn a_suffixed_key_that_cannot_be_made_unique_is_disclosed_not_overwritten() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                ("/a/b", json!({"get": op(json!({}))})),
                ("/a_b", json!({"get": op(json!({}))})),
                (
                    "/r",
                    json!({"get": op(json!({"operationId": "get_a_b_18cf5906"}))}),
                ),
                (
                    "/s",
                    json!({"get": op(json!({"operationId": "get_a_b_18cf5906f8a263bc"}))}),
                ),
            ],
        );
        let generated = generate(kind, &text).await;
        assert_eq!(
            generated.keys(),
            BTreeMap::from([
                ("get /a_b", "get_a_b_17523906"),
                ("get /r", "get_a_b_18cf5906"),
                ("get /s", "get_a_b_18cf5906f8a263bc"),
            ])
        );
        assert_eq!(generated.suggestions["get /a/b"], None);
        assert_eq!(
            generated.unsupported,
            BTreeMap::from([(
                "get /a/b".to_owned(),
                "openbindings.openapi.operation-key-unresolved".to_owned()
            )])
        );
    }
}

#[tokio::test]
async fn adding_an_unrelated_earlier_path_moves_no_existing_key() {
    for (kind, version) in EDITIONS {
        let existing = [
            (
                "/parcels",
                json!({
                    "get": op(json!({"operationId": "listParcels"})),
                    "post": op(json!({}))
                }),
            ),
            (
                "/parcels/{parcelId}",
                json!({
                    "get": op(json!({})),
                    "delete": op(json!({"operationId": "remove"}))
                }),
            ),
            (
                "/archive",
                json!({"delete": op(json!({"operationId": "remove"}))}),
            ),
            ("/a/b", json!({"get": op(json!({}))})),
            ("/a_b", json!({"get": op(json!({}))})),
            // Implementation-unsupported operations still hold their keys.
            (
                "/health",
                json!({"head": op(json!({"operationId": "health"}))}),
            ),
        ];
        let before = generate(kind, &document(version, &existing)).await;
        let mut extended = vec![(
            "/aardvarks",
            json!({"get": op(json!({})), "post": op(json!({"operationId": "addAardvark"}))}),
        )];
        extended.extend(existing.iter().cloned());
        let after = generate(kind, &document(version, &extended)).await;
        let mut expected = before.suggested();
        expected.insert("get /aardvarks", Some("get_aardvarks"));
        expected.insert("post /aardvarks", Some("addAardvark"));
        assert_eq!(after.suggested(), expected);
        assert_eq!(
            before.suggested(),
            BTreeMap::from([
                ("get /parcels", Some("listParcels")),
                ("post /parcels", Some("post_parcels")),
                ("get /parcels/{parcelId}", Some("get_parcels_parcelId")),
                (
                    "delete /parcels/{parcelId}",
                    Some("delete_parcels_parcelId")
                ),
                ("delete /archive", Some("delete_archive")),
                ("get /a/b", Some("get_a_b_18cf5906")),
                ("get /a_b", Some("get_a_b_17523906")),
                ("head /health", Some("health")),
            ])
        );
    }
}

#[tokio::test]
async fn implementation_unsupported_operations_take_part_in_uniqueness_and_collisions() {
    for (kind, version) in EDITIONS {
        let header = json!({"parameters": [
            {"name": "X-Trace", "in": "header", "schema": {"type": "string"}}
        ]});
        let text = document(
            version,
            &[
                ("/a/b", json!({"get": op(json!({}))})),
                // Unsupported here; it still collides with the supported `/a/b`.
                ("/a_b", json!({"get": op(header)})),
                (
                    "/items",
                    json!({
                        "get": op(json!({"operationId": "listItems"})),
                        // Outside the invocation profile; its identifier still counts.
                        "head": op(json!({"operationId": "listItems"}))
                    }),
                ),
            ],
        );
        let generated = generate(kind, &text).await;
        assert_eq!(
            generated.keys(),
            BTreeMap::from([
                ("get /a/b", "get_a_b_18cf5906"),
                ("get /items", "get_items")
            ])
        );
        // Inspection suggests keys for unsupported targets as well.
        assert_eq!(
            generated.suggested(),
            BTreeMap::from([
                ("get /a/b", Some("get_a_b_18cf5906")),
                ("get /a_b", Some("get_a_b_17523906")),
                ("get /items", Some("get_items")),
                ("head /items", Some("head_items")),
            ])
        );
        assert_eq!(
            generated.unsupported,
            BTreeMap::from([
                (
                    "get /a_b".to_owned(),
                    "openbindings.openapi.parameter-location-unimplemented".to_owned()
                ),
                (
                    "head /items".to_owned(),
                    "openbindings.openapi.unqualified-method".to_owned()
                ),
            ])
        );
    }
}

#[tokio::test]
async fn summary_and_description_compose_the_operation_description() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                (
                    "/both",
                    json!({"get": op(json!({"summary": "Ship a parcel", "description": "Creates a label.\nThen ships."}))}),
                ),
                (
                    "/summary",
                    json!({"get": op(json!({"summary": "Only a summary"}))}),
                ),
                (
                    "/description",
                    json!({"get": op(json!({"description": "Only a description"}))}),
                ),
                (
                    "/empty-summary",
                    json!({"get": op(json!({"summary": "", "description": "Described"}))}),
                ),
                (
                    "/empty-description",
                    json!({"get": op(json!({"summary": "Summarized", "description": ""}))}),
                ),
                (
                    "/empty",
                    json!({"get": op(json!({"summary": "", "description": ""}))}),
                ),
                (
                    // Path Item fields are never carried.
                    "/inherited",
                    json!({"summary": "Item summary", "description": "Item description", "get": op(json!({}))}),
                ),
            ],
        );
        let generated = generate(kind, &text).await;
        for (name, expected) in [
            (
                "get /both",
                json!({"description": "Ship a parcel\n\nCreates a label.\nThen ships."}),
            ),
            ("get /summary", json!({"description": "Only a summary"})),
            (
                "get /description",
                json!({"description": "Only a description"}),
            ),
            ("get /empty-summary", json!({"description": "Described"})),
            (
                "get /empty-description",
                json!({"description": "Summarized"}),
            ),
            ("get /empty", json!({})),
            ("get /inherited", json!({})),
        ] {
            assert_eq!(generated.metadata(name), expected, "{version} {name}");
        }
    }
}

#[tokio::test]
async fn deprecation_and_tags_are_carried_and_nothing_else() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[
                (
                    "/deprecated",
                    json!({"get": op(json!({"deprecated": true}))}),
                ),
                ("/current", json!({"get": op(json!({"deprecated": false}))})),
                (
                    "/tags",
                    json!({"get": op(json!({"tags": ["parcels", "admin", "parcels", "", "admin", ""]}))}),
                ),
                ("/untagged", json!({"get": op(json!({"tags": []}))})),
                (
                    "/documented",
                    json!({"get": op(json!({
                        "externalDocs": {"url": "https://docs.example.invalid/parcels"},
                        "parameters": [{"name": "q", "in": "query", "schema": {"type": "string"}, "example": "box"}],
                        "x-internal": true
                    }))}),
                ),
                (
                    "/all",
                    json!({"post": op(json!({
                        "summary": "Create",
                        "deprecated": true,
                        "tags": ["parcels"],
                        "operationId": "createParcel"
                    }))}),
                ),
            ],
        );
        let generated = generate(kind, &text).await;
        for (name, expected) in [
            ("get /deprecated", json!({"deprecated": true})),
            ("get /current", json!({})),
            ("get /tags", json!({"tags": ["parcels", "admin", ""]})),
            ("get /untagged", json!({})),
            ("get /documented", json!({})),
            (
                "post /all",
                json!({"description": "Create", "deprecated": true, "tags": ["parcels"]}),
            ),
        ] {
            assert_eq!(generated.metadata(name), expected, "{version} {name}");
        }
        assert_eq!(generated.operations["post /all"].0, "createParcel");
    }
}

#[tokio::test]
async fn metadata_of_the_wrong_type_is_not_carried() {
    for (kind, version) in EDITIONS {
        let text = document(
            version,
            &[(
                "/typed",
                json!({"get": op(json!({
                    "operationId": 7,
                    "summary": 7,
                    "description": "Described",
                    "deprecated": "true",
                    "tags": [1, "parcels", null, "parcels"]
                }))}),
            )],
        );
        let generated = generate(kind, &text).await;
        assert_eq!(
            generated.metadata("get /typed"),
            json!({"description": "Described", "tags": ["parcels"]})
        );
        assert_eq!(generated.operations["get /typed"].0, "get_typed");
    }
}
