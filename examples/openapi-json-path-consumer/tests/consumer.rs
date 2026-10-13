use dynamic_openapi_client::{Cancellation, HostCapabilities, PreparedRequest};
use openapi_json_path_consumer::*;
use openbindings_invocation::operation;
use openbindings_openapi::OpenApi;
use std::{cell::Cell, rc::Rc, sync::Mutex};

// Serialize this binary's tests because owner counters are process-global.
static OWNER_COUNTER_LOCK: Mutex<()> = Mutex::new(());

fn editions() -> [(&'static str, &'static str); 2] {
    [
        (openbindings_openapi::EXPERIMENTAL_KIND, DESCRIPTION),
        (openbindings_openapi::EXPERIMENTAL_KIND_30, DESCRIPTION_30),
    ]
}
async fn package_consumer_exact_roundtrip_and_owners() {
    async fn once(kind: &str, description: &str) {
        let saved = author(kind, description).await;
        let count = Rc::new(Cell::new(0));
        let recorded = count.clone();
        let provider = OpenApi::default().with_transport(
            move |request: PreparedRequest, cancel: Cancellation| {
                let recorded = recorded.clone();
                async move {
                    request
                        .invoke(
                            cancel,
                            &HostCapabilities::programmable(),
                            move |_, _| async move {
                                recorded.set(recorded.get() + 1);
                                reply(OUTPUT)
                            },
                        )
                        .await
                }
            },
        );
        let facade = invoker(&saved, provider);
        let parsed = openbindings::ParsedDocument::parse(&saved).unwrap();
        let draft = openbindings::DocumentBuilder::from_json(parsed.value()).unwrap();
        let name = draft.operations.keys().next().unwrap();
        assert_eq!(
            call(&facade, operation::Request::operation(name), INPUT)
                .await
                .unwrap()
                .as_deref(),
            Some(OUTPUT)
        );
        assert_eq!(count.get(), 1);
        assert_eq!(
            call(&facade, operation::Request::operation("missing"), INPUT)
                .await
                .unwrap_err()
                .code(),
            "ERR_OPERATION_NOT_FOUND"
        );
    }
    for (kind, description) in editions() {
        once(kind, description).await;
    }
    let before = owners();
    for _ in 0..1000 {
        for (kind, description) in editions() {
            once(kind, description).await;
        }
    }
    assert_eq!(owners(), before);
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "Each test owns its Tokio runtime."
)]
async fn lifecycle_mix_owners_return_to_baseline() {
    let _guard = OWNER_COUNTER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    package_consumer_exact_roundtrip_and_owners().await;
    use std::task::Poll;
    struct Guard(Rc<Cell<usize>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }
    async fn once(kind: &str, description: &str) {
        let saved = author(kind, description).await;
        let parsed = openbindings::ParsedDocument::parse(&saved).unwrap();
        let draft = openbindings::DocumentBuilder::from_json(parsed.value()).unwrap();
        let name = draft.operations.keys().next().unwrap();
        for malformed in [false, true] {
            let text = if malformed { "{" } else { OUTPUT };
            let provider = OpenApi::default().with_transport(
                move |request: PreparedRequest, cancel: Cancellation| async move {
                    request
                        .invoke(
                            cancel,
                            &HostCapabilities::programmable(),
                            move |_, _| async move { reply(text) },
                        )
                        .await
                },
            );
            assert_eq!(
                call(
                    &invoker(&saved, provider),
                    operation::Request::operation(name),
                    INPUT
                )
                .await
                .is_err(),
                malformed
            );
        }
        let choices = description.replace(
            r#"{"url":"https://consumer.invalid"}"#,
            r#"{"url":"https://consumer.invalid"},{"url":"https://other.invalid"}"#,
        );
        let choice_saved = author(kind, &choices).await;
        let provider = OpenApi::default().with_transport(|_: PreparedRequest, _: Cancellation| {
            std::future::pending::<dynamic_openapi_client::Outcome>()
        });
        assert_eq!(
            call(
                &invoker(&choice_saved, provider),
                operation::Request::operation(name),
                INPUT
            )
            .await
            .unwrap_err()
            .code(),
            "CONTEXT_REQUIRED"
        );
        for abandon in [false, true] {
            let live = Rc::new(Cell::new(0));
            let register = live.clone();
            let provider =
                OpenApi::default().with_transport(move |_: PreparedRequest, _: Cancellation| {
                    let register = register.clone();
                    async move {
                        register.set(register.get() + 1);
                        let _guard = Guard(register);
                        std::future::pending::<dynamic_openapi_client::Outcome>().await
                    }
                });
            let facade = invoker(&saved, provider);
            let control = openbindings::WorkControl::new();
            let mut job = facade.invoke_with_control(operation::Request::operation(name), &control);
            job.write(openbindings::JsonValue::parse(INPUT).unwrap())
                .await
                .unwrap();
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
        }
    }
    for (kind, description) in editions() {
        once(kind, description).await;
    }
    let before = owners();
    for _ in 0..1000 {
        for (kind, description) in editions() {
            once(kind, description).await;
        }
    }
    assert_eq!(owners(), before);
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "Each test owns its Tokio runtime."
)]
async fn authenticated_consumers_and_lifetimes() {
    let _guard = OWNER_COUNTER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    auth_acceptance::suite(false).await;
    auth_acceptance::suite(true).await;
    auth::additional_admission(false).await;
    auth::additional_admission(true).await;
    auth::bounds(false).await;
    auth::bounds(true).await;
    auth::suite(false).await;
    auth::suite(true).await;
    auth::cycle(false).await;
    auth::cycle(true).await;
    let before = owners();
    for _ in 0..1000 {
        auth::cycle(false).await;
        auth::cycle(true).await;
    }
    assert_eq!(owners(), before);
}
