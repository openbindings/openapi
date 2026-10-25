use super::*;
use dynamic_openapi_client::{
    Cancellation, Outcome,
    transport::{HostCapabilities, PreparedRequest},
};
use openbindings::Document;
use openbindings_source_inspection::CancellationToken;
use std::{cell::Cell, rc::Rc, task::Poll};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
#[wasm_bindgen]
pub async fn exact_roundtrip(
    kind: &str,
    description: &str,
    input: &str,
    response: &str,
) -> Result<String, JsValue> {
    let saved = author(kind, description).await;
    let output = response.to_owned();
    let calls = Rc::new(Cell::new(0));
    let recorded = calls.clone();
    let provider =
        OpenApi::default().with_transport(move |request: PreparedRequest, cancel: Cancellation| {
            let output = output.clone();
            let recorded = recorded.clone();
            async move {
                // A genuine JS suspension with an Rc host proves a local, non-Send seam.
                JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL))
                    .await
                    .unwrap();
                request
                    .invoke(
                        cancel,
                        &HostCapabilities::programmable(),
                        move |_, _| async move {
                            recorded.set(recorded.get() + 1);
                            reply(&output)
                        },
                    )
                    .await
            }
        });
    let facade = invoker(&saved, provider);
    let document = Document::parse(&saved).unwrap();
    let draft = &document;
    let name = draft.operations().unwrap().remove(0).key().to_owned();
    let result = call(&facade, operation::Request::operation(&name), input)
        .await
        .map_err(|e| JsValue::from_str(e.code()))?;
    assert_eq!(calls.get(), 1);
    Ok(result.unwrap())
}
#[wasm_bindgen]
pub fn logical_owners() -> String {
    let (sdk, client) = owners();
    format!("{sdk},{client}")
}
#[wasm_bindgen]
pub async fn crud_consumers() {
    crate::crud::suite(true).await;
    crate::crud::suite(false).await;
}
/// Received responses and followed redirects for both kinds; the number of host calls.
#[wasm_bindgen]
pub async fn received_responses() -> u32 {
    let mut entries = 0;
    for v30 in [false, true] {
        entries += crate::responses::suite(v30).await;
    }
    u32::try_from(entries).unwrap()
}
#[wasm_bindgen]
pub async fn pending_cleanup(kind: &str, description: &str, abandon: bool) -> Result<(), JsValue> {
    struct Guard(Rc<Cell<usize>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }
    let saved = author(kind, description).await;
    let live = Rc::new(Cell::new(0));
    let registered = live.clone();
    let provider = OpenApi::default().with_transport(
        move |_request: PreparedRequest, _cancel: Cancellation| {
            let registered = registered.clone();
            async move {
                registered.set(registered.get() + 1);
                let _guard = Guard(registered);
                let promise = js_sys::Promise::new(&mut |_resolve, _reject| {});
                let _ = JsFuture::from(promise).await;
                std::future::pending::<Outcome>().await
            }
        },
    );
    let facade = invoker(&saved, provider);
    let document = Document::parse(&saved).unwrap();
    let draft = &document;
    let name = draft.operations().unwrap().remove(0).key().to_owned();
    let control = CancellationToken::new();
    let mut job = facade.invoke_with_control(operation::Request::operation(&name), &control);
    job.write(JsonValue::parse(INPUT).unwrap()).await.unwrap();
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
    if !abandon {
        control.cancel();
        assert_eq!(
            job.next().await.unwrap().unwrap_err().code(),
            "ERR_CANCELLED"
        );
    }
    drop(job);
    assert_eq!(live.get(), 0);
    Ok(())
}

#[wasm_bindgen]
pub async fn authenticated_consumers() -> Result<(), JsValue> {
    crate::auth_acceptance::suite(false).await;
    crate::auth_acceptance::suite(true).await;
    super::auth::additional_admission(false).await;
    super::auth::additional_admission(true).await;
    super::auth::bounds(false).await;
    super::auth::bounds(true).await;
    super::auth::suite(false).await;
    super::auth::suite(true).await;
    super::auth::cycle(false).await;
    super::auth::cycle(true).await;
    let before = owners();
    for _ in 0..1000 {
        super::auth::cycle(false).await;
        super::auth::cycle(true).await;
    }
    assert_eq!(owners(), before);
    Ok(())
}

/// Exact JS text context enters the same interface path; report actual effects on errors.
#[wasm_bindgen]
pub async fn authenticated_call(
    v30: bool,
    path: &str,
    method: &str,
    context_text: &str,
    input: &str,
) -> String {
    let (kind, text, _, _) = super::auth::edition(v30);
    let src = super::auth::source(kind, text);
    let seen = Rc::new(super::auth::Seen::default());
    let result = match JsonValue::parse(context_text) {
        Ok(context) => {
            super::auth::binding_call(
                OpenApi::default().with_transport(super::auth::host(
                    seen.clone(),
                    200,
                    super::auth::OUTPUT,
                )),
                &src,
                path,
                method,
                Some(&context),
                &[input],
            )
            .await
        }
        Err(_) => Err(InvocationError::refused()),
    };
    match result {Ok(output)=>serde_json::json!({"output":output,"code":null,"hostEntries":seen.calls.get(),"wires":*seen.wires.borrow()}).to_string(),Err(error)=>serde_json::json!({"output":[],"code":error.code(),"hostEntries":seen.calls.get(),"wires":*seen.wires.borrow()}).to_string()}
}
