//! Public Rust consumer. Host instrumentation is qualification-only.
use openbindings::{Binding, DocumentBuilder, JsonValue, Operation, Source};
use openbindings_openapi::{EXPERIMENTAL_KIND, OpenApi};
use openbindings_source_inspection::{BindableTarget, Inspector};
use std::collections::BTreeMap;

pub fn source(content: &str) -> Result<Source, openbindings::InputError> {
    Ok(Source {
        kind: EXPERIMENTAL_KIND.into(),
        content: Some(JsonValue::parse(content)?),
        ..Default::default()
    })
}

/// Author-owned keys intentionally differ from operationId/sourceRef suggestions.
pub fn author(source: Source, binding: Binding) -> Result<JsonValue, Box<dyn std::error::Error>> {
    let mut draft = DocumentBuilder::new();
    draft
        .operations
        .insert("catalog.lookup".into(), Operation::default());
    draft.sources = Some(BTreeMap::from([("catalog-source".into(), source)]));
    draft.bindings = Some(BTreeMap::from([("catalog-binding".into(), binding)]));
    let parsed = draft.build()?;
    let assessment = parsed.assess()?;
    if assessment.validated().is_none() {
        return Err("authored OBI does not conform".into());
    }
    let encoded = draft.to_json()?;
    let reloaded = DocumentBuilder::from_json(&encoded)?;
    assert_eq!(
        reloaded.bindings.as_ref().unwrap()["catalog-binding"].operation,
        "catalog.lookup"
    );
    Ok(encoded)
}

pub fn author_selected(
    source: Source,
    target: BindableTarget,
) -> Result<JsonValue, Box<dyn std::error::Error>> {
    author(
        source,
        target.binding.bind("catalog.lookup", "catalog-source")?,
    )
}

/// Choose an explicit mounted target; partiality remains an application decision.
pub async fn inspect_and_author(
    content: &str,
    target_ref: &str,
) -> Result<JsonValue, Box<dyn std::error::Error>> {
    let source = source(content)?;
    let provider = OpenApi::default();
    let inventory = Inspector::new(&provider).inspect(&source).await?;
    if !inventory.exhaustive {
        return Err("inventory is partial; inspect limitation before choosing".into());
    }
    let target = inventory
        .targets
        .into_iter()
        .find(|t| t.source_ref == target_ref)
        .ok_or("selected target absent")?;
    drop(provider);
    author_selected(source, target)
}

#[cfg(all(target_arch = "wasm32", feature = "wasm-probe"))]
mod wasm {
    use super::*;
    use openbindings::WorkControl;
    use std::rc::Rc;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen(js_name=inspectSource)]
    pub async fn inspect_source(content: String, cancelled: bool) -> Result<String, JsValue> {
        let source = source(&content).map_err(|e| JsValue::from_str(&e.to_string()))?;
        drop(content);
        let provider = Rc::new(OpenApi::default());
        let control = WorkControl::new();
        if cancelled {
            control.cancel();
        }
        let result = Inspector::new(provider.clone())
            .inspect_with_control(&source, &control)
            .await
            .map_err(|e| JsValue::from_str(e.code()))?;
        drop(provider);
        drop(source);
        result
            .to_json()
            .map(|v| v.text().to_owned())
            .map_err(|e| JsValue::from_str(e.code()))
    }
    #[wasm_bindgen(js_name=authorSource)]
    pub async fn author_source(content: String, target: String) -> Result<String, JsValue> {
        inspect_and_author(&content, &target)
            .await
            .map(|v| v.text().to_owned())
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }
    #[wasm_bindgen(js_name=liveClientOwners)]
    pub fn client_owners() -> usize {
        dynamic_openapi_client::live_json_owners()
    }
    #[wasm_bindgen(js_name=liveSdkOwners)]
    pub fn sdk_owners() -> usize {
        openbindings_internal_json::backend::live_arenas()
    }
}
