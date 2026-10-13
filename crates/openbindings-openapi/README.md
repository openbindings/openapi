# Experimental OpenAPI adapter

This optional crate implements the existing source inspector, interface synthesizer,
and binding invoker roles for two private experiments in one `OpenApi<T>` provider:

| Source kind | OpenAPI family | Exported kind constant | Main fixture version |
| --- | --- | --- | --- |
| `local.openapi-json-probe.r1` | 3.1 | `EXPERIMENTAL_KIND` | 3.1.2 |
| `local.openapi30-json-probe.r1` | 3.0 | `EXPERIMENTAL_KIND_30` | 3.0.4 |

Use `openbindings-invocation::operation::Invoker` for document-based operation selection
and declared value contracts. Core and the published roles do not depend on OpenAPI.
This package does not implement a published OpenAPI binding specification.

Both use the same finite JSON request profile and public role APIs. The source kind
must match the description's edition, including when reusing a prepared source.
Patch-family admission is not a claim of testing every patch or every capability.
Shared-subset 3.0.0/3.0.3 smoke cases complement the main 3.0.4 fixtures.
Inspection and synthesis require no transport. Bind application-owned execution once
with `OpenApi::with_transport` to invoke. Local non-`Send` hosts work on Wasm; native
hosts retain `Send` when their futures and captured state are `Send`.

## Source and binding

```json
{
  "kind": "local.openapi-json-probe.r1",
  "content": {
    "document": "{\"openapi\":\"3.1.2\", ...}",
    "retrievalUri": "https://descriptions.example/api.json"
  }
}
```

`document` is JSON **text**, preserving numbers beyond JavaScript's exact integer
range. `retrievalUri` is an optional absolute resource identity. It grants no fetch
permission. Put additional `ResourceInput` values in `Options.resources`. Saving an
OBI preserves its source; independent cold reload must re-supply external resources
at the same identities. Untyped fragments also need the client's explicit
`ResourceLayout` interpretation roots. That is a supplied-resource admission
choice in this implementation, not a required mechanism for a future kind.
The OBI does not embed the whole resource graph.

A binding's content is an exact entry path and lowercase method:

```json
{"path":"/parcels/{parcelId}","method":"get"}
```

Mounted paths identify callable operations. Component pointers, `operationId`, and
suggested generated names do not. Generated core keys may be renamed consistently.

## Invocation

Send exactly one JSON object, then close input. No request is dispatched until input
closes. Zero or additional values, unknown envelope members, undeclared parameters,
duplicate JSON names, or unsupported shapes refuse before effects.

```json
{"parameters":{"path":{"parcelId":"box-7"},"query":{"parcelId":"alias-9"}}}
```

```json
{"body":{"tracking":9007199254740993,"label":"雪"}}
```

An absent `body` means no entity; `"body": null` sends JSON null. Path and query scalar
values remain separate. Supported JSON success produces one exact body value,
including one value for JSON null. A selected successful response declaration
with no content produces no value only when delivered bytes are verified empty.
Exact status declarations take precedence over `2XX`, then `default`. HTTP 204
and 205 always require empty delivery. An empty JSON-declared response fails;
unexpected bytes on a content-less response fail rather than being discarded.
An unmatched successful status retains the experiment's JSON-success fallback;
this is not complete response-conformance validation.
Requiring empty bytes when the selected declaration omits content is this
experiment's policy, not an OpenAPI guarantee about every server. A content-less
`default` also governs unmatched 2xx responses and can therefore cause a failure
after dispatch if the server supplies a payload. The private r1 labels are retained
for this capability extension and the HTTP 205 correction; the latter also rejects
illegal 205 payloads previously returned as JSON. These response mappings belong to the private experiment.
Always drain and check the terminal outcome; receiving
an output alone does not establish success. Complete non-2xx responses produce
the kind's status error with data `{"status":404}` and no output:
`HTTP_STATUS_ERROR` for 3.1 or `HTTP_STATUS_ERROR_30` for 3.0.
Malformed, incomplete or unsupported successful delivery produces `ERR_EXECUTION_FAILED`.
Cancellation and other post-dispatch failures do not promise rollback or authorize retry.

The adapter constructs finite requests under `Options` limits, enters `Transport::execute`
at most once per attempt, follows no redirects, and never replays after any outcome.
It attaches only explicitly selected per-attempt credentials with caller-approved
origins; it acquires or attaches no ambient credentials. These are adapter guarantees.

A `Transport` implementation must enforce the same restrictions in its HTTP client:
disable automatic redirects, replay/retries, ambient authentication and cookie jars,
and preserve the prepared target and headers. It must perform its own capability
admission before dispatch, using `PreparedRequest::invoke` with honest
`HostCapabilities` to run the client's strict checks before the HTTP callback.
The adapter does not obtain or check that declaration, and declarations alone do
not prove the underlying HTTP client's behavior. Hosts also own honest outcome
evidence, cancellation and cleanup.

**SSRF:** server URLs come from the OpenAPI document. A host invoking untrusted
documents must restrict destinations, applying policy at every network hop after
DNS resolution, including proxy connections. Declared-server membership and
credential-origin grants do not authorize access to a destination or network range.
Destination policy remains a host duty even for an anonymous operation.

If effective server URLs are ambiguous, the adapter returns the existing `config.value`
challenge. Supply the selected declared URL in
`context.configuration["local.openapi-json-probe.r1.server"]` and deliberately start a
new attempt. For the 3.0 kind use `SERVER_POINT_30`, whose value is
`local.openapi30-json-probe.r1.server`. Each kind uses its own context point;
the other kind's point does not select a server. Context applies to that attempt;
the adapter neither stores choices nor
retries. URLs select declared destinations and do not allow an arbitrary override.
Malformed or undeclared server choices are refused before effects; only an absent
choice can trigger a server challenge.
The opaque challenge `target` distinguishes retrieval identity, operation and effective
choices and kind. It is not a trusted cache key or authority; applications own persistence.

The maintained [external consumer](../../examples/openapi-json-path-consumer) shows
inspection, synthesis, save/reload, explicit evaluator setup, and a finite call helper
that preserves the actual terminal even if `write` reports closed admission. Its thin
TypeScript probe passes JSON as text through Wasm. It is qualification code, not a
published first-class TypeScript adapter API.

## Authenticated JSON calls

The two private kinds support HTTP bearer tokens and API keys declared in headers.
The application supplies already obtained credentials through the existing role
context; it owns acquisition, storage, trust, TLS and transport policy. The adapter
never logs in, refreshes, retries, verifies token claims, or caches credentials.

```json
{
  "credentials": { "ServiceToken": "application-supplied-token" },
  "configuration": {
    "local.openapi-json-probe.r1.security": {
      "schemes": ["ServiceToken"],
      "origins": { "ServiceToken": ["https://api.example.test"] }
    }
  }
}
```

Pass this value with the existing `operation::Request::with_context` or
`BindingRequest.context`. The constants are `SECURITY_POINT` and `SECURITY_POINT_30`;
the latter names `local.openapi30-json-probe.r1.security`. Ordinary operation inputs
remain parameters/body. Neither the OBI nor synthesis needs or stores secrets.

`schemes` is an exact set of source-declared names, with no duplicate names or array
ordering requirement. A single distinct security alternative can be inferred;
multiple alternatives require selection even if only one credential is supplied.
`[]` selects a declared anonymous alternative. All schemes in a chosen alternative
apply together. Unselected credentials are never forwarded. Equivalent duplicate
alternatives share a choice; there is no fallback to another choice after failure.
Named `credentials` is the primary API. Existing `bearerToken`, `apiKey`, and
`apiKeys[name]` inputs work when unambiguous; conflicting applicable values refuse.
API-key strings are opaque: empty strings are preserved, while the client's HTTP
field validation rejects malformed values. An empty bearer token is invalid under
that mechanism. The adapter does not judge whether a server will accept a key.

Each selected credential needs caller-approved `origins`. Scheme/host/effective-port
comparison follows the client; it checks the actual request destination too. Source
URLs and challenge examples are untrusted assertions, never permission or a suggested
grant. Retrieval identity does not authorize release. Applications must decide which
origins are appropriate independently. Explicit nonmatching grants refuse rather
than prompting to widen access. Custom hosts remain trusted application code.

Missing destination is challenged first, then missing security selection, then
credentials/grants. Supply corrected context in a deliberate new invocation. No
host is entered before a challenge/refusal; an HTTP 401/403 is a status error after
dispatch and never a request to retry automatically. The default advisory preflight
does not prove authentication readiness.

Authenticated profiles admit the **whole encoded context** under `Options.context_bytes`
(default 64 KiB), including unrelated fields; duplicate names in that context refuse.
Already-unsecured profiles continue to ignore authentication context. Public security
metadata and incrementally constructed challenges are bounded by `result_bytes`.
The generic invocation facade separately applies `max_error_bytes`: an oversized
portable error becomes `ERR_EXECUTION_FAILED`, with zero host entries in this path,
under its existing behavior. No truncated challenge is offered as a usable subset.
These are implementation policies, not requirements imposed on generic interfaces.

Basic, OAuth/OIDC schemes, mTLS, query/cookie keys, nonempty bearer/key role arrays,
and statically unusable credential-header combinations remain unsupported. Any
unsupported effective alternative excludes that operation as a whole in this profile.
This includes case-insensitive collisions, invalid HTTP field names, and keys using
Content-Type on an operation with a body. Other supported operations survive.
Synthesis reports runtime prerequisites under the security point and keeps
AND/OR structure in nonsecret coverage details. Missing runtime credentials are not
schema loss or a reason to exclude an otherwise supported operation.

## Synthesis and scope

Zero sources produce a skeleton; one source is preserved; composition of multiple
sources is currently unsupported. Generated input schemas describe the mechanical
request envelope, scalar parameter names/types and required flags. Supply a schema
evaluator to the generic operation invoker when invoking those declared contracts.
Source schema constraints and output schemas are not translated in this slice:
coverage reports projection loss. Callable targets, unsupported known targets, and
unestablished inventory are distinguished. Strict eligibility is not full schema fidelity.

For 3.0, nullable scalar parameters and parameter Schema Reference Objects are
currently implementation-unsupported. A 3.0 Reference Object's sibling `type` does
not establish a parameter type. The client resolves protocol references for
parameters, request bodies, responses and supplied Path Items using the source
edition. Body schemas remain unprojected: 3.0 `nullable` and 3.1 null unions do not
become copied OBI schemas. JSON null bodies retain their distinct entity meaning.
These limitations do not declare the corresponding OpenAPI features invalid.
For 3.1 parameter types, this implementation accepts the OAS 3.1 base dialect
(`https://spec.openapis.org/oas/3.1/dialect/base` or its `2024-11-10` identifier)
and JSON Schema 2020-12. The client's lexical schema context determines the dialect,
including schema-level overrides of `jsonSchemaDialect`. Other dialects produce
`parameter-schema-dialect-unimplemented`; their raw `type` cannot establish a caller
contract here. This guard does not translate schemas or validate source-schema constraints.
OpenAPI 3.0 GET and DELETE request-body declarations are ignored under that edition's rules;
they neither require nor enable an input body. OpenAPI 3.1 GET and DELETE body declarations
remain effective. The paired consumer regression verifies this distinction.

The supported profile uses GET/POST/PUT/PATCH/DELETE, absolute HTTP(S) servers without variables,
path/query scalar parameters, finite JSON bodies, and supplied references. Forms,
security acquisition, binary/streaming delivery, richer parameter locations, callbacks,
webhook services, ambiguous Path Item merges, and literal non-ASCII path templates
are not implemented. HEAD, OPTIONS and TRACE remain outside this implementation's
invocation profile. PATCH uses application/json; JSON Patch and Merge Patch media
types remain unsupported. Percent-encoded Unicode paths preserve their spelling; the client gap
for literal Unicode is tracked separately. Bodies/headers on explicit non-2xx responses
are discarded under the status-only mapping and their lost representation is disclosed
in coverage. A `default` response can also cover success, so it must meet the supported
success profile; an `application/problem+json` default makes the operation
implementation-unsupported. Content-less exact, range and default declarations
are supported. The [CRUD consumer](../../examples/openapi-json-path-consumer/src/crud.rs)
exercises both manually authored and synthesized documents through binding and
operation invocation, including an authenticated PATCH.

`Options` owns immutable client/graph budgets and supplied resources. The adapter
caches one prepared source by kind and content and charges retained metadata. Alternating
sources on one provider re-analyzes each source. A cache miss evicts the previous
entry, then analysis runs outside the cache lock before inserting its bounded result.
Concurrent misses may compute identical results more than once; the last completed
analysis replaces the single entry. A failed analysis retains no new entry.
Warm timings apply to repeated use of
the same source. A provider per source avoids alternating-source cache misses when
that matters to the application. No cache lock is held across host I/O.
Source, result, input and body limits are finite. Logical budgets are not RSS
measurements. Host policy belongs to the application; the qualified reqwest companion
follows no redirects and performs no implicit retry or credential acquisition.

The consumer verification entry point is `python3 tools/verify-openapi-json-path.py --wasm`
from the repository root. Archive qualification additionally requires
`--archives --sdk-source <cached-sdk-checkout>` at the pinned revision.
Run `cargo test --locked --offline` for the full adapter suite, including native HTTP.
Native HTTP and Node/Wasm seam qualification do not establish browser or Worker network
support, package publication, or full OpenAPI coverage.
