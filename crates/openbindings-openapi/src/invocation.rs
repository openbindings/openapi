use crate::{Kind, OpenApi, analysis, json, redirect, response};
use dynamic_openapi_client::{
    Cancellation, Outcome, ParameterLocation, RequestBuilder, exact::Value, inspect::Method,
    transport::PreparedRequest,
};
use futures_core::Stream;
use openbindings::{JsonKind, JsonRef, JsonValue, WorkControl};
use openbindings_invocation::{
    BindingRequest, CancellationToken, ContextChallenge, InvocationError, ProviderInput,
    ProviderIo, ProviderOutput,
    binding::{BindingInvoker, SupportedKind},
};
use std::{
    borrow::Cow,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Application-owned execution of one finite prepared request.
///
/// The adapter waits for closed input, bounds request construction through
/// [`crate::Options`], and calls [`Self::execute`] once for the request and once for
/// each redirect it follows, each a separate call. It derives the request that follows
/// a redirect with [`PreparedRequest::follow_redirect`], at most `Limits::redirects`
/// times (20 by default), and checks cancellation between hops. It never replays a
/// request (including after a status or transport failure) and attaches no ambient
/// credentials: only explicitly selected, per-attempt credentials with caller-approved
/// origins are supplied, and none of them follows a redirect to another origin.
/// These guarantees cover the adapter, not HTTP behavior inside the host.
///
/// Implementations must not follow redirects themselves: disable automatic redirects,
/// replay/retries and ambient authentication (including cookies), apply their own
/// capability admission before dispatch, and preserve the prepared target and
/// headers. Dispatch through [`PreparedRequest::invoke`] with honest
/// [`dynamic_openapi_client::transport::HostCapabilities`] so the client's strict
/// admission runs before the HTTP callback. A capability declaration is not proof that
/// the underlying HTTP client obeys it; the adapter does not obtain or check the host's
/// declaration itself. Report a redirect as the received 3xx with its complete header
/// section, so the adapter can follow it.
///
/// Destination policy is also a host duty, at every hop: a redirect the adapter follows
/// reaches [`Self::execute`] as a new request, and the host applies its admission and
/// destination policy to it as to any other. Check each network hop, including the
/// resolved addresses after DNS resolution and any proxy connection. Server URLs and
/// redirect locations come from the OpenAPI document and the server and are untrusted;
/// hosts invoking untrusted documents must restrict destinations to prevent SSRF.
/// Declared-server membership and credential-origin grants do not authorize network
/// access.
///
/// Content codings are a host duty too. The adapter removes no content coding and adds
/// no `Accept-Encoding` header, since browser and Worker Fetch hosts forbid setting it.
/// A host sends an `Accept-Encoding` that names only the codings it removes (`identity`
/// when it removes none), or lets its platform own codings, as Fetch does. It reports
/// [`Provenance::Decoded`] or [`Provenance::HostDelivered`] for delivered bytes whose
/// codings are removed; a body that still carries a coding listed in `Content-Encoding`,
/// reported as [`Provenance::Wire`] or [`Provenance::Unknown`], completes unsuccessfully.
///
/// Implementations must report honest client outcome evidence and own
/// cancellation/cleanup. No executor or Send bound is imposed. A closure taking
/// an owned plan can implement this trait and has the same duties.
///
/// [`Provenance::Decoded`]: dynamic_openapi_client::transport::Provenance::Decoded
/// [`Provenance::HostDelivered`]: dynamic_openapi_client::transport::Provenance::HostDelivered
/// [`Provenance::Wire`]: dynamic_openapi_client::transport::Provenance::Wire
/// [`Provenance::Unknown`]: dynamic_openapi_client::transport::Provenance::Unknown
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

type Emit = Pin<Box<dyn Future<Output = Result<(), InvocationError>> + Send>>;
fn emit(mut output: ProviderOutput, value: JsonValue) -> Emit {
    Box::pin(async move { output.emit(value).await })
}

/// Provider job preserving conditional Send: a local host stays local; a Sync
/// host with Send execution futures can be used by native multithreaded callers.
/// Dropping it cancels its client token and drops pending host work.
pub struct InvocationFuture<'a, T: Transport> {
    provider: &'a OpenApi<T>,
    request: BindingRequest<'a>,
    input: Option<ProviderInput>,
    value: Option<JsonValue>,
    output: Option<ProviderOutput>,
    execution: Option<Pin<Box<T::Execution<'a>>>>,
    /// The request the pending execution sends; the redirect step derives the next
    /// request from it and its outcome.
    sent: Option<PreparedRequest>,
    emitting: Option<Emit>,
    control: openbindings_invocation::Cancellation,
    cancellation: Cancellation,
    work: CancellationToken,
    /// Whether the request sent is a HEAD request, whose response carries no content.
    head: bool,
    done: bool,
}
impl<T: Transport> InvocationFuture<'_, T> {
    /// Enter the transport with `request`, a separate call per hop, keeping the request
    /// for the redirect step, which needs the request that produced each outcome.
    fn send(&mut self, request: PreparedRequest) {
        self.head = matches!(request.method(), Method::Head);
        self.sent = Some(request.clone());
        self.execution = Some(Box::pin(
            self.provider
                .transport
                .execute(request, self.cancellation.clone()),
        ));
    }
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
                let value = match Pin::new(input).poll_next(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(value) => value,
                };
                if let Some(value) = value {
                    if this.value.is_some() {
                        this.input = None;
                        break Poll::Ready(Err(InvocationError::refused()));
                    }
                    this.value = Some(value);
                    continue;
                }
                this.input = None;
                // One request from the first value, or from no value when input closed first.
                let value = this.value.take();
                let prepared = match prepare_request(
                    this.provider,
                    this.request,
                    value.as_ref(),
                    &this.work,
                    &this.cancellation,
                ) {
                    Ok(p) => p,
                    Err(e) => break Poll::Ready(Err(e)),
                };
                if this.work.is_cancelled() {
                    break Poll::Ready(Err(InvocationError::cancelled()));
                }
                this.send(prepared);
            }
            if let Some(execution) = &mut this.execution {
                let outcome = match execution.as_mut().poll(cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(outcome) => outcome,
                };
                this.execution = None;
                let sent = this
                    .sent
                    .take()
                    .expect("the request that produced the outcome");
                if outcome.cancelled() {
                    break Poll::Ready(Err(InvocationError::cancelled()));
                }
                match redirect::step(&sent, &outcome) {
                    Ok(next) => {
                        // Cancellation is checked between hops, before the transport is
                        // entered again.
                        if this.work.is_cancelled() {
                            break Poll::Ready(Err(InvocationError::cancelled()));
                        }
                        this.send(next);
                        continue;
                    }
                    Err(redirect::End::Final) => {}
                    Err(redirect::End::Failed) => {
                        break Poll::Ready(Err(InvocationError::execution_failed()));
                    }
                }
                let limits = &this.provider.options.limits;
                let value = match response::read(&outcome, this.head, limits) {
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
            .map(|kind| {
                SupportedKind::new(kind.label()).with_description(
                    "Private kind, not a published standard; no implicit retry or resource acquisition",
                )
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
            input: Some(input),
            value: None,
            output: Some(output),
            execution: None,
            sent: None,
            emitting: None,
            control: work.cancelled(),
            cancellation: Cancellation::default(),
            work,
            head: false,
            done: false,
        }
    }
}
fn prepare_request<T>(
    provider: &OpenApi<T>,
    request: BindingRequest<'_>,
    value: Option<&JsonValue>,
    control: &dyn WorkControl,
    cancellation: &Cancellation,
) -> Result<PreparedRequest, InvocationError> {
    let refused = || InvocationError::refused();
    let p = provider.prepare(request.source, control).map_err(|_| {
        if control.is_cancelled() {
            InvocationError::cancelled()
        } else {
            refused()
        }
    })?;
    let binding = request.binding.content.as_ref().ok_or_else(refused)?;
    // `path` and `method` name the target. The kinds' value adaptation members, `input`,
    // `output` and `failure`, are refused here with any other member: no adaptation
    // evaluator is adopted yet.
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
    let mut input = unit.operation.request().cancellation(cancellation.clone());
    // With no `input` adaptation, the operation's input value is the native request. No
    // value is the native request with nothing in it: no parameters and no content.
    if let Some(value) = value {
        input = native_request(provider, profile, value, control, input)?;
    }
    // Every invocation that gets here reads a supplied context, since `<kind>.server`
    // applies even with one server. The context is admitted whole before any point is read.
    let context = request
        .context
        .map(|c| admit_context(c, &provider.options))
        .transpose()?;
    let configuration = context.and_then(|c| c.get("configuration"));
    let point = |name: &str| configuration.and_then(|c| c.get(name));
    // The challenge target distinguishes the kind, source location, operation and choices.
    let identity = |choices: &[&str]| {
        serde_json_compat::json!([
            p.kind.label(),
            request
                .source
                .content
                .as_ref()
                .and_then(|v| v.get("location"))
                .and_then(JsonRef::as_str),
            path,
            method,
            choices
        ])
        .to_string()
    };
    let Some((server, choice)) =
        crate::server::choose(&profile.servers, point(p.kind.server_point()))?
    else {
        // Several effective servers and no choice: the caller chooses among the declared
        // URLs this implementation can use.
        let choices: Vec<&str> = profile
            .servers
            .iter()
            .filter(|s| s.usable())
            .map(|s| s.url.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        return Err(config_challenge(
            identity(&choices),
            p.kind.server_point(),
            "/url",
            &choices,
        ));
    };
    input = crate::server::apply(server, &choice, input);
    if let Some(body) = value.and_then(|v| v.get("body")) {
        let media = profile.body.as_ref().ok_or_else(refused)?;
        let Some(elected) = media.elect(point(p.kind.request_media_point()))? else {
            let choices: Vec<&str> = media
                .choices()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            return Err(config_challenge(
                identity(&choices),
                p.kind.request_media_point(),
                "",
                &choices,
            ));
        };
        input = input.media(elected).json_exact(
            &Value::parse(body.text(), provider.options.limits.clone()).map_err(|_| refused())?,
        );
    }
    // Only an authenticated profile needs the destination origin, which a templated
    // server resolves for this choice.
    let origin = if profile.security.iter().any(|a| !a.is_empty()) {
        server.origin(&choice)?
    } else {
        Cow::Borrowed("")
    };
    input = crate::security::apply(
        profile,
        context,
        crate::security::Target {
            kind: p.kind,
            path,
            method,
            origin: &origin,
        },
        &provider.options,
        control,
        input,
    )?;
    dynamic_openapi_client::advanced::prepare(input)
        .0
        .map_err(|_| refused())
}
/// A context the adapter reads is admitted whole, whichever point is read: within
/// `Options.context_bytes`, and with no member name repeated at any depth. Both are
/// bounded queries of the parsed value, not walks of it.
fn admit_context<'c>(
    context: &'c JsonValue,
    options: &crate::Options,
) -> Result<&'c JsonValue, InvocationError> {
    if context.bytes().len() > options.context_bytes || context.has_duplicate_names() {
        return Err(InvocationError::refused());
    }
    Ok(context)
}
/// Check the native request's members and give its parameters to the builder.
fn native_request<T>(
    provider: &OpenApi<T>,
    profile: &analysis::Profile,
    value: &JsonValue,
    control: &dyn WorkControl,
    mut input: RequestBuilder,
) -> Result<RequestBuilder, InvocationError> {
    let refused = || InvocationError::refused();
    analysis::object(value.view(), &["path", "query", "header", "cookie", "body"])
        .map_err(|_| refused())?;
    // Duplicate names anywhere in the value, parameter names included, refuse.
    if value.has_duplicate_names() {
        return Err(refused());
    }
    // This profile declares no header or cookie parameter, so a member in either
    // location names nothing declared; an empty object contributes nothing.
    for location_name in ["header", "cookie"] {
        if let Some(values) = value.get(location_name)
            && values.members().ok_or_else(refused)?.next().is_some()
        {
            return Err(refused());
        }
    }
    ["path", "query"]
        .into_iter()
        .filter_map(|name| value.get(name))
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
        if let Some(values) = value.get(location_name) {
            let members = values.members().ok_or_else(refused)?;
            for member in members {
                let name = member.name.as_str().ok_or_else(refused)?;
                if control.is_cancelled() {
                    return Err(InvocationError::cancelled());
                }
                let expected = profile
                    .parameters
                    .get(location_name)
                    .and_then(|parameters| parameters.get(name))
                    .ok_or_else(refused)?;
                // Strings and booleans are sent as written; a number as the canonical
                // decimal numeral of its exact value, within the client's numeral limit.
                let text = match member.value.kind() {
                    JsonKind::String if expected == "string" => Cow::Borrowed(member.value.text()),
                    JsonKind::Boolean if expected == "boolean" => {
                        Cow::Borrowed(member.value.text())
                    }
                    JsonKind::Number if expected == "number" || expected == "integer" => {
                        let canonical = crate::number::canonical(
                            member.value.number_text().ok_or_else(refused)?,
                            provider.options.limits.number_conversion_chars,
                        )
                        .ok_or_else(refused)?;
                        if expected == "integer" && !canonical.integer {
                            return Err(refused());
                        }
                        canonical.text
                    }
                    _ => return Err(refused()),
                };
                let value = Value::parse(text.as_bytes(), provider.options.limits.clone())
                    .map_err(|_| refused())?;
                input = input.parameter_exact(location, name, &value);
            }
        }
    }
    if value.get("body").is_some() && profile.body.is_none() {
        return Err(refused());
    }
    Ok(input)
}
/// A `config.value` challenge for one configuration point, offering its usable values.
fn config_challenge(target: String, point: &str, path: &str, choices: &[&str]) -> InvocationError {
    let shape = json(
        &serde_json_compat::json!({"target":target,"alternatives":[{"requirements":[{"type":"config.value","point":point,"path":path,"schema":{"type":"string","enum":choices}}]}]}),
    );
    shape
        .ok()
        .and_then(|v| ContextChallenge::new(v).ok())
        .map(InvocationError::challenge)
        .unwrap_or_else(InvocationError::refused)
}
