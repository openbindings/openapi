use crate::{Kind, OpenApi, analysis, json};
use dynamic_openapi_client::{
    Cancellation, Outcome, ParameterLocation,
    exact::Value,
    transport::{BodyState, HeaderDelivery, PreparedRequest},
};
use openbindings::{JsonKind, JsonRef, JsonValue, WorkControl};
use openbindings_invocation::{
    BindingRequest, CancellationToken, ContextChallenge, InvocationError, ProviderInput,
    ProviderIo, ProviderOutput,
    binding::{BindingInvoker, SupportedKind},
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Application-owned execution of one finite prepared request.
///
/// The adapter waits for closed input, bounds request construction through
/// [`crate::Options`], and calls [`Self::execute`] at most once per attempt. It
/// follows no redirects, never replays a request (including after a status or
/// transport failure), and attaches no ambient credentials: only explicitly
/// selected, per-attempt credentials with caller-approved origins are supplied.
/// These guarantees cover the adapter, not HTTP behavior inside the host.
///
/// Implementations must disable automatic redirects, replay/retries and ambient
/// authentication (including cookies), apply their own capability admission
/// before dispatch, and preserve the prepared target and headers. Dispatch through
/// [`PreparedRequest::invoke`] with honest [`dynamic_openapi_client::transport::HostCapabilities`]
/// so the client's strict admission runs before the HTTP callback. A capability
/// declaration is not proof that the underlying HTTP client obeys it; the adapter
/// does not obtain or check the host's declaration itself.
///
/// Destination policy is also a host duty: check each network hop, including the
/// resolved addresses after DNS resolution and any proxy connection. Server URLs
/// come from the OpenAPI document and are untrusted; hosts invoking untrusted
/// documents must restrict destinations to prevent SSRF. Declared-server membership
/// and credential-origin grants do not authorize network access.
///
/// Implementations must report honest client outcome evidence and own
/// cancellation/cleanup. No executor or Send bound is imposed. A closure taking
/// an owned plan can implement this trait and has the same duties.
pub trait Transport {
    /// A host future that may borrow its reusable host.
    type Execution<'a>: Future<Output = Outcome>
    where
        Self: 'a;
    /// Execute once; ownership of the plan avoids self-referential futures.
    fn execute(&self, request: PreparedRequest, cancellation: Cancellation) -> Self::Execution<'_>;
}
impl<F, Fut> Transport for F
where
    F: Fn(PreparedRequest, Cancellation) -> Fut,
    Fut: Future<Output = Outcome>,
{
    type Execution<'a>
        = Fut
    where
        Self: 'a;
    fn execute(&self, request: PreparedRequest, cancellation: Cancellation) -> Fut {
        self(request, cancellation)
    }
}

type NextInput = Pin<Box<dyn Future<Output = (ProviderInput, Option<JsonValue>)> + Send>>;
type Emit = Pin<Box<dyn Future<Output = Result<(), InvocationError>> + Send>>;
fn receive(mut input: ProviderInput) -> NextInput {
    Box::pin(async move {
        let value = input.next().await;
        (input, value)
    })
}
fn emit(mut output: ProviderOutput, value: JsonValue) -> Emit {
    Box::pin(async move { output.emit(value).await })
}

/// Provider job preserving conditional Send: a local host stays local; a Sync
/// host with Send execution futures can be used by native multithreaded callers.
/// Dropping it cancels its client token and drops pending host work.
pub struct InvocationFuture<'a, T: Transport> {
    provider: &'a OpenApi<T>,
    request: BindingRequest<'a>,
    input: Option<NextInput>,
    value: Option<JsonValue>,
    output: Option<ProviderOutput>,
    execution: Option<Pin<Box<T::Execution<'a>>>>,
    emitting: Option<Emit>,
    control: openbindings_invocation::Cancellation,
    cancellation: Cancellation,
    work: CancellationToken,
    empty_responses: analysis::EmptyResponses,
    done: bool,
}
impl<T: Transport> Unpin for InvocationFuture<'_, T> {}
impl<T: Transport> Drop for InvocationFuture<'_, T> {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
impl<T: Transport> Future for InvocationFuture<'_, T> {
    type Output = Result<(), InvocationError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        assert!(!this.done, "polled completed adapter job");
        let result = loop {
            if Pin::new(&mut this.control).poll(cx).is_ready() {
                this.cancellation.cancel();
                break Poll::Ready(Err(InvocationError::cancelled()));
            }
            if let Some(input) = &mut this.input {
                let (input, value) = match input.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(value) => value,
                };
                this.input = None;
                if let Some(value) = value {
                    if this.value.is_some() {
                        break Poll::Ready(Err(InvocationError::refused()));
                    }
                    this.value = Some(value);
                    this.input = Some(receive(input));
                    continue;
                }
                drop(input);
                let Some(value) = this.value.take() else {
                    break Poll::Ready(Err(InvocationError::refused()));
                };
                let (prepared, empty_responses) = match prepare_request(
                    this.provider,
                    this.request,
                    &value,
                    &this.work,
                    &this.cancellation,
                ) {
                    Ok(p) => p,
                    Err(e) => break Poll::Ready(Err(e)),
                };
                this.empty_responses = empty_responses;
                if this.work.is_cancelled() {
                    break Poll::Ready(Err(InvocationError::cancelled()));
                }
                this.execution = Some(Box::pin(
                    this.provider
                        .transport
                        .execute(prepared, this.cancellation.clone()),
                ));
            }
            if let Some(execution) = &mut this.execution {
                let outcome = match execution.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(outcome) => outcome,
                };
                this.execution = None;
                let kind =
                    Kind::from_label(&this.request.source.kind).expect("admitted source kind");
                let value = match accept(outcome, kind, this.empty_responses) {
                    Ok(value) => value,
                    Err(e) => break Poll::Ready(Err(e)),
                };
                if let Some(value) = value {
                    this.emitting =
                        Some(emit(this.output.take().expect("one output owner"), value));
                } else {
                    break Poll::Ready(Ok(()));
                }
            }
            if let Some(emitting) = &mut this.emitting {
                break emitting.as_mut().poll(cx);
            }
        };
        if result.is_ready() {
            this.done = true;
        }
        result
    }
}
impl<T: Transport> BindingInvoker for OpenApi<T> {
    type Invocation<'a>
        = InvocationFuture<'a, T>
    where
        Self: 'a;
    fn supports_kind(&self, kind: &str) -> bool {
        Kind::from_label(kind).is_some()
    }
    fn supported_kinds(&self) -> Vec<SupportedKind> {
        Kind::ALL
            .into_iter()
            .map(|kind| SupportedKind {
                kind: kind.label().into(),
                description: Some(
                    "Private finite JSON experiment; no implicit retry or resource acquisition"
                        .into(),
                ),
            })
            .collect()
    }
    fn invoke<'a>(
        &'a self,
        request: BindingRequest<'a>,
        io: ProviderIo,
        work: CancellationToken,
    ) -> Self::Invocation<'a> {
        let (input, output) = io.split();
        InvocationFuture {
            provider: self,
            request,
            input: Some(receive(input)),
            value: None,
            output: Some(output),
            execution: None,
            emitting: None,
            control: work.cancelled(),
            cancellation: Cancellation::default(),
            work,
            empty_responses: Default::default(),
            done: false,
        }
    }
}
fn prepare_request<T>(
    provider: &OpenApi<T>,
    request: BindingRequest<'_>,
    value: &JsonValue,
    control: &dyn WorkControl,
    cancellation: &Cancellation,
) -> Result<(PreparedRequest, analysis::EmptyResponses), InvocationError> {
    let refused = || InvocationError::refused();
    let p = provider.prepare(request.source, control).map_err(|_| {
        if control.is_cancelled() {
            InvocationError::cancelled()
        } else {
            refused()
        }
    })?;
    let binding = request.binding.content.as_ref().ok_or_else(refused)?;
    analysis::object(binding.view(), &["path", "method"]).map_err(|_| refused())?;
    let path = binding
        .get("path")
        .and_then(JsonRef::as_str)
        .ok_or_else(refused)?;
    let method = binding
        .get("method")
        .and_then(JsonRef::as_str)
        .ok_or_else(refused)?;
    let unit = p
        .units
        .iter()
        .find(|u| {
            u.operation.path() == path && u.operation.method().as_str().eq_ignore_ascii_case(method)
        })
        .ok_or_else(refused)?;
    if method != method.to_ascii_lowercase() {
        return Err(refused());
    }
    let profile = unit.profile.as_ref().map_err(|_| refused())?;
    analysis::object(value.view(), &["parameters", "body"]).map_err(|_| refused())?;
    if value.has_duplicate_names() {
        return Err(refused());
    }
    let mut input = unit.operation.request().cancellation(cancellation.clone());
    if let Some(parameters) = value.get("parameters") {
        analysis::object(parameters, &["path", "query"]).map_err(|_| refused())?;
        ["path", "query"]
            .into_iter()
            .filter_map(|name| parameters.get(name))
            .try_fold(0usize, |n, value| {
                let mut total = n;
                for _ in value.members().ok_or_else(refused)? {
                    if control.is_cancelled() {
                        return Err(InvocationError::cancelled());
                    }
                    total = total.checked_add(1).ok_or_else(refused)?;
                    if total > provider.options.limits.reference_steps {
                        return Err(refused());
                    }
                }
                Ok(total)
            })?;
        for (location, location_name) in [
            (ParameterLocation::Path, "path"),
            (ParameterLocation::Query, "query"),
        ] {
            if let Some(values) = parameters.get(location_name) {
                let members = values.members().ok_or_else(refused)?;
                let mut names = std::collections::HashSet::new();
                for member in members {
                    let name = member.name.as_str().ok_or_else(refused)?;
                    if !names.insert(name) {
                        return Err(refused());
                    }
                    if control.is_cancelled() {
                        return Err(InvocationError::cancelled());
                    }
                    let expected = profile
                        .parameters
                        .get(location_name)
                        .and_then(|parameters| parameters.get(name))
                        .ok_or_else(refused)?;
                    let valid = match member.value.kind() {
                        JsonKind::String => expected == "string",
                        JsonKind::Boolean => expected == "boolean",
                        JsonKind::Number => {
                            expected == "number"
                                || (expected == "integer"
                                    && integer(member.value.number_text().expect("number")))
                        }
                        _ => false,
                    };
                    if !valid {
                        return Err(refused());
                    }
                    let value = Value::parse(member.value.text(), provider.options.limits.clone())
                        .map_err(|_| refused())?;
                    input = input.parameter_exact(location, name, &value);
                }
            }
        }
    }
    if let Some(body) = value.get("body") {
        if !profile.has_body {
            return Err(refused());
        }
        input = input.json_exact(
            &Value::parse(body.text(), provider.options.limits.clone()).map_err(|_| refused())?,
        );
    }
    let servers = &profile.servers;
    let server_point = p.kind.server_point();
    let choice = request
        .context
        .and_then(|c| c.get("configuration"))
        .and_then(|c| c.get(server_point));
    let selected = match choice {
        Some(choice) => {
            let choice = choice.as_str().ok_or_else(refused)?;
            Some(
                servers
                    .iter()
                    .position(|candidate| candidate == choice)
                    .ok_or_else(refused)?,
            )
        }
        None if servers.iter().all(|s| s == &servers[0]) => Some(0),
        None => None,
    };
    let selected = selected.ok_or_else(|| {
        let choices: std::collections::BTreeSet<_> = servers.iter().map(|s| s.as_str()).collect();
        let target = serde_json_compat::json!([p.kind.label(), request.source.content.as_ref().and_then(|v| v.get("retrievalUri")).and_then(JsonRef::as_str), path, method, choices]).to_string();
        let shape = json(&serde_json_compat::json!({"target":target,"alternatives":[{"requirements":[{"type":"config.value","point":server_point,"path":"","schema":{"type":"string","enum":choices}}]}]}));
        shape.ok().and_then(|v| ContextChallenge::new(v).ok()).map(InvocationError::challenge).unwrap_or_else(refused)
    })?;
    input = input.server(selected);
    input = crate::security::apply(
        profile,
        request.context,
        crate::security::Target {
            kind: p.kind,
            path,
            method,
            server: &servers[selected],
        },
        &provider.options,
        control,
        input,
    )?;
    dynamic_openapi_client::advanced::prepare(input)
        .0
        .map(|request| (request, profile.empty_responses))
        .map_err(|_| refused())
}
fn accept(
    outcome: Outcome,
    kind: Kind,
    empty_responses: analysis::EmptyResponses,
) -> Result<Option<JsonValue>, InvocationError> {
    if outcome.cancelled() {
        return Err(InvocationError::cancelled());
    }
    // A host outcome is evidence, not permission to infer absence of effects.
    if outcome.error().is_some()
        || outcome.dispatch() != dynamic_openapi_client::transport::DispatchEvidence::Dispatched
    {
        return Err(InvocationError::execution_failed());
    }
    let status = outcome
        .status()
        .ok_or_else(InvocationError::execution_failed)?;
    if outcome.body_state() != Some(BodyState::Complete)
        || outcome.header_delivery() != Some(HeaderDelivery::Complete)
        || outcome.omitted_header_lines() != Some(0)
    {
        return Err(InvocationError::execution_failed());
    }
    if !(200..300).contains(&status) {
        let data = json(&serde_json_compat::json!({"status":status}))
            .map_err(|_| InvocationError::execution_failed())?;
        return Err(InvocationError::new(kind.status_error(), Some(data))
            .expect("fixed governed error shape"));
    }
    if matches!(status, 204 | 205) || empty_responses.contains(status) {
        return if outcome.raw().is_some_and(<[u8]>::is_empty) {
            Ok(None)
        } else {
            Err(InvocationError::execution_failed())
        };
    }
    let decoded = outcome
        .received_2xx_json_exact()
        .map_err(|_| InvocationError::execution_failed())?;
    JsonValue::parse(decoded.source())
        .map(Some)
        .map_err(|_| InvocationError::execution_failed())
}

// Exact integrality without machine-number conversion or exponent expansion.
fn integer(token: &str) -> bool {
    let token = token.trim_start_matches('-');
    let (mantissa, exponent) = token.split_once(['e', 'E']).unwrap_or((token, "0"));
    if mantissa.bytes().all(|b| b == b'0' || b == b'.') {
        return true;
    }
    let fractional = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
    let zeros = mantissa
        .bytes()
        .filter(|b| *b != b'.')
        .rev()
        .take_while(|b| *b == b'0')
        .count();
    let exponent = exponent
        .parse::<i64>()
        .unwrap_or(if exponent.starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        });
    exponent >= fractional as i64 - zeros as i64
}
