//! Compile-only consumer: a host has already supplied the adapter and binding.
//! No value-schema evaluator, reqwest, TypeScript runtime, or executor is required.
use openbindings::{JsonValue, Source};
use openbindings_invocation::{BindingRequest, BindingValue, InvocationError, binding};
use openbindings_openapi::{OpenApi, Transport};
pub async fn invoke<T: Transport>(
    provider: &OpenApi<T>,
    source: &Source,
    binding: &BindingValue,
    input: JsonValue,
) -> Result<Vec<JsonValue>, InvocationError> {
    let facade = binding::Invoker::new(provider);
    let mut job = facade.invoke(BindingRequest {
        source,
        binding,
        context: None,
    });
    let _ = job.write(input).await;
    job.close();
    let mut values = Vec::new();
    while let Some(value) = job.next().await {
        values.push(value?);
    }
    job.outcome().expect("drained terminal")?;
    Ok(values)
}
