//! Compile-only consumer: a host has already supplied the adapter, source and binding.
//! The input is the operation's native request, an object with optional `path`, `query`,
//! `header`, `cookie` and `body` members, and a successful call emits zero or one value.
//! No value-schema evaluator, reqwest, TypeScript runtime, or executor is required.
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding};
use openbindings_openapi::{OpenApi, Transport};
pub async fn invoke<T: Transport>(
    provider: &OpenApi<T>,
    source: &Source,
    binding: &BindingValue,
    request: JsonValue,
) -> Result<Option<JsonValue>, InvocationError> {
    let facade = binding::Invoker::new(provider);
    let mut job = facade.invoke(BindingRequest {
        source,
        binding,
        context: None,
    });
    let _ = job.write(request).await;
    job.close();
    let mut value = None;
    while let Some(next) = job.next().await {
        if value.replace(next?).is_some() {
            return Err(InvocationError::execution_failed());
        }
    }
    job.outcome().expect("drained terminal")?;
    Ok(value)
}
