# OpenAPI adapter

This optional crate implements the existing source inspector, interface synthesizer,
and binding invoker interfaces for two private kinds in one `OpenApi<T>` provider:

| Source kind | Documents | Exported constant | Main fixture version |
| --- | --- | --- | --- |
| `local.openapi-3.1.r2` | `openapi` 3.1.x, read under OpenAPI 3.1.2 | `KIND_31` | 3.1.2 |
| `local.openapi-3.0.r2` | `openapi` 3.0.x, read under OpenAPI 3.0.4 | `KIND_30` | 3.0.4 |

The kinds are defined by this project for its own use. They are not a published binding
standard, and this package does not implement a published OpenAPI binding specification.
The earlier private labels `local.openapi-json-probe.r1` and `local.openapi30-json-probe.r1`
are retired: their meaning stays frozen, they are never reused, and inspection, synthesis
and invocation refuse them before any work, exactly as they refuse any other unknown kind.

Use `openbindings-invocation::operation::Invoker` for document-based operation selection
and declared value contracts. Construct it with the `Conformant<Document>` returned
by `document.validate()?.conformant()` after checking that proof exists. The checked
synthesizer already returns conformance proof; saving and reloading requires fresh
validation. A conformant operation's `input()` and `output()` hand out its value
contracts, prepared with `prepare(&evaluator)` or `prepare_with(&evaluator, &resources)`;
invoker `with_evaluator` is infallible. `PreparedValueContract::validate` is the cheap check:
a failure carries a fixed message, no problems and incomplete detail. `explain` returns
the same verdict with the library's first error. The invoker withholds a rejected value
on its verdict; `with_validation_detail(Detail::Explain)` makes its validation failure
carry that explanation. See
[the migration record](../../PROVENANCE.md) for diagnostic and reference behavior.
Core and the published interfaces do not depend on OpenAPI.

Both kinds share one implementation profile and the public interface APIs. The source
kind must match the description's edition, including when reusing a prepared source.
Patch-family admission is not a claim of testing every patch or every capability.
Shared-subset 3.0.0/3.0.3 smoke cases complement the main 3.0.4 fixtures.
Inspection and synthesis require no transport. Bind application-owned execution once
with `OpenApi::with_transport` to invoke. Local non-`Send` hosts work on Wasm; native
hosts retain `Send` when their futures and captured state are `Send`.

## Kind contract

This section states what the two kinds mean as this implementation applies them. Where
the kinds allow something this implementation does not handle yet, it refuses before any
request, and each part below says what it refuses for now. One rule waits on a client
change; see [content codings on the native host](#content-codings-on-the-native-host).

### Source content

```json
{
  "kind": "local.openapi-3.1.r2",
  "content": {
    "document": {"openapi": "3.1.2", "info": {"title": "Parcels", "version": "1"}, "paths": {}},
    "location": "https://descriptions.example/api.json"
  }
}
```

`content` is an object with `document`, `location` or both, and no other member.

- `document` is the entry document, embedded either as a JSON object or as a string of
  JSON text. An embedded object reaches the client as that member's own exact JSON text,
  never a re-serialization, so its numbers keep their exact values, including integers
  beyond JavaScript's exact range. A string is read as its decoded text.
- `location` is an absolute URI: the entry document's identity and the base for its
  relative references and relative server URLs. It grants no fetch permission.

Refused for now, as unsupported and before any request:

- a `document` string holding YAML (`openbindings.openapi.yaml-document-unimplemented`).
  The kinds admit YAML; this implementation reads JSON only;
- a `location` without a `document` (`openbindings.openapi.document-retrieval-unimplemented`).
  This implementation never fetches, and bytes supplied in `Options.resources` at that
  identity do not stand in for `document`.

Any other member, a `document` that is neither an object nor a string, or a `location`
that is not an absolute URI (a scheme and no fragment, RFC 3986 section 4.3) is invalid
content. An absolute URI the client cannot use as a document identity, such as one with
user information, is refused as unsupported.

Put additional `ResourceInput` values in `Options.resources`. Saving an OBI preserves its
source; independent cold reload must re-supply external resources at the same identities.
Untyped fragments also need the client's explicit `ResourceLayout` interpretation roots.
That is a supplied-resource admission choice in this implementation, not a required
mechanism of the kinds. The OBI does not embed the whole resource graph.

### Binding content

A binding's content is an exact Paths key and a lowercase Path Item method key:

```json
{"path":"/parcels/{parcelId}","method":"get"}
```

Mounted paths identify callable operations. Component pointers, `operationId`, and
generated operation keys do not; [synthesis](#synthesis-and-scope) states how keys
are generated.

The kinds also define the optional value adaptation members `input`, `output` and
`failure`, written in JSONata. No evaluator is adopted yet, so a binding carrying any of
them is refused before any request. Any other member is refused.

### The native request

Without an `input` adaptation, the operation's input value is the native request: an
object whose optional members are the request's parts. Parameters are keyed by name
within their OpenAPI location, so the same name in two locations stays distinct.

```json
{"path":{"parcelId":"box-7"},"query":{"parcelId":"alias-9"}}
```

```json
{"body":{"tracking":9007199254740993,"label":"雪"}}
```

- `path` and `query` are objects naming declared parameters with scalar values of their
  declared type. An undeclared name, a parameter in another location, or another JSON
  type refuses before any request, and so does a non-integral number for an `integer`.
- A boolean parameter is written `true` or `false`. A number parameter is written as the
  plain decimal numeral of its exact value: an optional `-`, digits, and a fraction only
  when the value is not an integer, with no trailing zeros and no exponent. `1e2` and
  `100.0` are written `100`, `1.50` is `1.5`, `1E-2` is `0.01` and `-0` is `0`. The text
  comes from the exact JSON token, never a binary float. A numeral longer than the
  client's `Limits::number_conversion_chars` (256 by default), sign and point included,
  refuses before any request. A JSON body still carries its JSON text exactly.
- `header` and `cookie` parameters are outside the supported profile, so no operation
  here declares one: a non-empty `header` or `cookie` member names nothing declared and
  refuses. An empty object contributes nothing.
- An absent `body` sends no content; `"body": null` sends JSON null. A `body` on an
  operation that declares no request body refuses.
- A null parameter value and a duplicate member name anywhere in the value refuse. The
  kinds allow both refusals.
- Any other member refuses, including the earlier `parameters` wrapper.

One invocation makes one request, from its first input value, or from no value when
input closes first. No value is the native request with nothing in it: no parameters and
no content, with the usual checks, so a required parameter or request body is still
required. Nothing is dispatched until input closes; a second value refuses before effects.

### Servers

The effective servers are the operation's, else the Path Item's, else the document's.
None, or an empty list, means one server at `/`. One effective server is used.

- A relative server URL resolves against the `location` of the document that declares
  it. The implied `/` resolves against the `location` of the document that declares the
  empty list, or against the source's `location` when no list is declared. A relative server with
  no `location` cannot be used, and no challenge is offered for it: the only choice that
  could satisfy it is a `baseUrl`, which this implementation refuses.
- A variable takes the value supplied for it, else its `default`. A supplied value
  outside a declared `enum`, a non-string value, or a name the chosen server does not
  declare refuses before any request. A declared variable the URL does not use changes
  nothing.
- With several effective servers, the caller chooses one at `<kind>.server`. Without a
  choice the result is a `config.value` challenge naming that point with path `/url` and
  an `enum` of the declared URLs this implementation can use.

The `<kind>.server` value is an object with optional members:

```json
{"url": "https://{region}.api.example/v1", "variables": {"region": "eu"}}
```

- `url` names a declared server by its URL exactly as written. It is needed only when
  there are several effective servers; when given, it must name one.
- `variables` is an object of string values keyed by variable name. It applies with one
  server too.
- `baseUrl`, an absolute `http` or `https` base URL of the caller's own, exclusive of the
  other two, is accepted by the kinds but refused before any request here: the client
  has no API for it yet.

A choice selects a declared destination; it is not an arbitrary override. A malformed or
undeclared choice refuses before effects, and only an absent choice raises the server
challenge. Several declarations of one URL count once when their variables are identical;
with different variables, a URL cannot tell them apart, so that URL is not usable. Refused
or unusable here, before any request: a server URL with a query or fragment, one that
does not reach an `http` or `https` origin, a template variable without a declaration, a
declaration without a string `default`, or an `enum` that is empty, holds a non-string
or omits the default. An operation none of whose effective servers can be used is
implementation-unsupported (`openbindings.openapi.server-profile-unimplemented`).

Server URLs come from the document and are untrusted, so their analysis is linear in the
declarations. Each distinct list is analyzed once per prepared source and shared by every
operation whose effective list it is; what it retains is charged to `result_bytes` once,
as it is built, and cancellation is checked between declarations.

### Security selection

A supplied choice wins: `schemes` at `<kind>.security` is an exact set of scheme names,
and `[]` selects the anonymous alternative `{}`. Otherwise, of the alternatives whose
every scheme has a credential in context:

- exactly one that needs credentials is used;
- with none, the anonymous alternative is used when one is declared;
- with several, the caller chooses: the challenge offers each alternative's `schemes`.

When nothing is satisfied and nothing is anonymous, the challenge asks for the missing
credentials, one challenge alternative per security alternative, any one of which would
do. A scheme has a credential when `credentials[name]` is present, or one of the
interface's conveniences applies to it: `apiKeys[name]` for an API key, and the flat
`bearerToken` or `apiKey` for an alternative with exactly one scheme of that kind. Equal
alternatives count once. [Authenticated calls](#authenticated-calls) describes the
credentials, origin grants and refusals that apply to the alternative used.

### Request media

When a body is supplied, its media is the operation's one declared media type. With
several, it is the one JSON type when there is exactly one. Only concrete JSON keys count,
`application/json` or a `+json` type, and keys that differ only in letter case, such as
`application/json` and `Application/JSON`, count once. A declared range resolves to
`application/json` only when it is the operation's one declared key, or when the caller
chooses it. So `[application/vnd.acme+json, */*]` sends `application/vnd.acme+json`,
`[*/*]` sends `application/json`, and `[text/plain, */*]` asks the caller rather than
sending JSON unasked. Otherwise the caller names one declared media key, exactly as
written, at `<kind>.requestMedia`; without a choice the result is a `config.value`
challenge at that point with path `""` and an `enum` of the declared keys this
implementation can send, ranges included.

Only JSON media are implemented. A choice naming no declared key, or a key whose media
is not JSON, refuses before any request. The client matches `application/json` and
`Application/JSON` alike, so a body for an operation declaring both refuses before any
request too, whether or not a key is chosen. A request body none of whose keys is a JSON
type or a range matching `application/json` is implementation-unsupported
(`openbindings.openapi.request-media-unimplemented`), as is one whose JSON media declare
an `encoding`. Without a body no media is chosen and `<kind>.requestMedia` is not
consulted. JSON Patch and Merge Patch are `+json` media types, so a PATCH body in either
is sent as its JSON text.

In the 3.0 kind a request body on GET, HEAD or DELETE has no meaning: it is never sent, a
supplied `body` refuses, and the synthesized input schema has no `body`. In the 3.1 kind
such a body is sent when declared.

### Outcomes

The received response decides: the description's response declarations never change how
a response is read. A final 2xx succeeds, declared or not, when its evidence is complete:
the request was dispatched with no execution error and no cancellation, the body is
complete, and header delivery is `HeaderDelivery::Complete` with no header omitted.
Truncation, a transport failure or abandonment completes unsuccessfully with no value.
A successful response emits at most one value.

- **No content emits no value:** a 204, a response to HEAD, and any other 2xx whose
  content is zero octets. HTTP framing gives a 204 and a response to HEAD no content, so
  bytes a host delivers with one complete unsuccessfully. A 205 must have no content: an
  empty 205 emits no value, and a 205 with content completes unsuccessfully.
- **Otherwise the one `Content-Type` decides the value:**
  - JSON media, `application/json` or any `+json` type with any parameters, become the
    JSON value;
  - text media, `text/*` or any other type with a `charset` parameter, become a JSON
    string;
  - anything else becomes a Base64 string (RFC 4648 section 4, padded) of the octets;
  - no `Content-Type` is read as `application/octet-stream`: Base64, with no sniffing;
  - more than one `Content-Type` field, or one that does not parse as a single media type
    under RFC 9110 section 8.3.1 (a list does not parse), completes unsuccessfully.
- **Text** with no `charset`, or with `charset=utf-8` in any case, is read as UTF-8.
  `charset=us-ascii` is read as US-ASCII, so every octet must be below 0x80. A leading
  UTF-8 byte order mark is not part of the string. Text that is not valid in its charset,
  any other charset (aliases such as `utf8` included) and a repeated `charset` parameter
  complete unsuccessfully: the kinds read text in UTF-8 and US-ASCII only.
- **JSON** is parsed once, from the delivered bytes, and every number keeps its exact
  token text. A leading UTF-8 byte order mark is ignored. Duplicate member names take the
  last occurrence, at every depth: the value holds each name once, where its last
  occurrence stood, and only a value that has duplicates is rebuilt. Invalid JSON,
  including an escape denoting an unpaired surrogate, completes unsuccessfully.
- **Content codings.** `Content-Encoding` lists codings in the order applied, and
  `identity` is no coding. This implementation removes no coding itself. A body still
  carrying a coding, which the host reports as `Provenance::Wire` or `Provenance::Unknown`,
  completes unsuccessfully, even when it is zero octets. Bytes the host reports as
  `Provenance::Decoded`, or `Provenance::HostDelivered` from a Fetch host whose platform
  removes codings, are read as delivered. The adapter adds no `Accept-Encoding` header;
  [invocation details](#invocation-details) state the host's duty.
- **Headers never become values.** Nothing from a response header reaches the emitted
  value. Under the kinds a header reaches a value only through the binding's adaptation,
  and none is supported yet.

A text or Base64 value, and JSON parsing, stay within the SDK's default JSON limits
(32 MiB); JSON is also bounded by the client's `body_bytes`, `depth` and `nodes` limits,
a member name and its value counting as up to two nodes.

A final non-2xx response completes unsuccessfully with `ERR_EXECUTION_FAILED` and no data.
Under the kinds, a protocol outcome reaches the operation only through the binding's
adaptation, and no adaptation is supported yet: neither the status nor the body of a
non-2xx reaches the caller, whatever the description declares for it. That includes a 3xx
that is not followed, which [redirects](#redirects) describe, and an HTTP 401 or 403,
which is never a request to retry automatically.

Always drain and check the terminal outcome; receiving an output alone does not establish
success. Cancellation and other post-dispatch failures do not promise rollback or
authorize retry.

### Redirects

A 301, 302, 303, 307 or 308 with a `Location` is followed, up to 20 times. The client
derives each next request with `PreparedRequest::follow_redirect`, and the adapter sends it
through the same `Transport::execute` as a separate call, so every hop passes the host's
admission and destination policy. The client applies the rules; the adapter does not
repeat them:

- **Methods and content.** 303 becomes GET without content (HEAD stays HEAD); 301 and 302
  turn POST into GET without content; otherwise the method and content are repeated.
  Dropping the content also drops `Content-Type`, `Content-Encoding`, `Content-Language`
  and `Content-Location`.
- **Target.** `Location` resolves against the request's target as a URI reference, and
  its fragment is dropped.
- **Credentials.** On the same origin (scheme, host and effective port) the request keeps
  its credentials. On another origin every credential the request carries is removed,
  wherever it was applied, together with every `Authorization` and `Cookie` field, and it
  does not return on a later hop back to the original origin.
- **Limit.** At most `Limits::redirects` redirects are followed, taken from
  `Options.limits`: 20 by default, the Fetch standard's limit.

Cancellation is checked between hops, and dropping the invocation still cancels the hop in
progress. Each hop is a new request the server asked for, not a replay.

A redirect that is not followed:

- **An https to http redirect** is not followed. That 3xx is the final response and is
  read under the outcome rules; with no failure adaptation supported yet, it completes
  unsuccessfully with no data.
- **A 300, 304 or 305, or a redirect with no `Location`,** is the final response, and
  completes unsuccessfully with no data.
- **A redirect beyond the limit** completes unsuccessfully, as the Fetch standard's
  network error does.
- **An unusable `Location`** completes unsuccessfully: one that is repeated, is not UTF-8
  or not a URI reference, or does not resolve to an absolute `http` or `https` URI with a
  host and no userinfo.
- **Incomplete header evidence** completes unsuccessfully: following needs
  `HeaderDelivery::Complete` with no header omitted, so a truncated header list cannot
  hide or forge a `Location`.
- **A browser's opaque redirect** completes unsuccessfully. A browser Fetch host hides
  the redirect, exposing neither its status nor its `Location`, so the adapter can neither
  follow it nor read it as the final response. Letting the platform follow it instead
  would skip the credential and downgrade rules above, so a host that cannot observe a
  redirect makes the call unsuccessful: a limitation of browser hosts, not a setting.
  Native and custom hosts deliver the real 3xx. The client documents, from its own
  workerd probe, that its Worker Fetch profile does too; this adapter has not been run on
  a Worker host.

### Configuration points

Each kind names its configuration points by its own label, so one context can serve
bindings of both kinds:

| Point | 3.1 kind constant | 3.0 kind constant |
| --- | --- | --- |
| `<kind>.server` | `SERVER_POINT_31` (`local.openapi-3.1.r2.server`) | `SERVER_POINT_30` |
| `<kind>.security` | `SECURITY_POINT_31` (`local.openapi-3.1.r2.security`) | `SECURITY_POINT_30` |
| `<kind>.requestMedia` | `REQUEST_MEDIA_POINT_31` (`local.openapi-3.1.r2.requestMedia`) | `REQUEST_MEDIA_POINT_30` |

The other kind's points, and the retired labels' points, select nothing. A missing
destination is challenged first, then the request media, then security selection, then
credentials and their origin grants. No host is entered before a challenge or refusal.
Context applies to the one attempt it is passed with: supply corrected context in a
deliberate new invocation. The adapter neither stores choices nor retries. A challenge's
opaque `target` distinguishes the source `location`, operation, effective choices and
kind; it is not a trusted cache key or authority, and applications own persistence.

### Content codings on the native host

The kinds require a request to name only the content codings that can be removed:
`Accept-Encoding: identity` when none can. Fetch hosts' platforms own codings. The client's
reqwest companion does not send `Accept-Encoding` yet and reports `Provenance::Wire`, so a
server may answer in any coding, and a coded response completes unsuccessfully. The
companion sending `Accept-Encoding: identity` when the caller set none is a separate client
change; until this adapter pins it, a native host meets the rule by sending the header
itself.

## Invocation details

The adapter reads the delivered bytes itself rather than through the client's
received-2xx JSON read, which refuses duplicate member names and every media type but
JSON. Response acceptance requires affirmative `HeaderDelivery::Complete` and zero
core header omissions, including for empty and non-2xx responses. The client's
received-2xx reads also accept `Unspecified`; this adapter keeps its stricter
policy. The adapter checks cancellation before an execution diagnostic,
so their coexistence still produces `ERR_CANCELLED`. The client's ordinary read
instead returns the actual diagnostic first; both refuse success. Other execution
diagnostics, and a response that cannot become a value, map to `ERR_EXECUTION_FAILED`,
while request construction diagnostics map to `ERR_REFUSED`. These mappings expose no arbitrary
diagnostic text or credential material. Upload evidence remains independent.

The adapter constructs finite requests under `Options` limits, enters `Transport::execute`
once for the request and once for each redirect it follows, and never replays after any
outcome. It attaches only explicitly selected per-attempt credentials with caller-approved
origins, never sends them to another origin on a redirect, and acquires or attaches no
ambient credentials. These are adapter guarantees.

A `Transport` implementation must enforce the same restrictions in its HTTP client. It
must not follow redirects itself: the adapter follows them by repeated `execute` calls,
and a host that followed them would bypass the credential rules and the chain limit. So it
disables automatic redirects, replay/retries, ambient authentication and cookie jars,
preserves the prepared target and headers, and reports a redirect as the received 3xx
with its complete header section. It must perform its own capability
admission before dispatch, using `PreparedRequest::invoke` with honest
`HostCapabilities` to run the client's strict checks before the HTTP callback.
The adapter does not obtain or check that declaration, and declarations alone do
not prove the underlying HTTP client's behavior. Hosts also own honest outcome
evidence, cancellation and cleanup.

Content codings are a host duty too. The adapter removes no coding and adds no
`Accept-Encoding` header, since browser and Worker Fetch hosts forbid setting it and the
adapter cannot tell its host. A host sends an `Accept-Encoding` naming only the codings
it removes (`identity` when it removes none), or lets its platform own codings, as Fetch
does, and reports `Provenance::Decoded` or `Provenance::HostDelivered` for bytes whose
codings it removed. The native companion's current behavior is described under
[content codings on the native host](#content-codings-on-the-native-host).

Since the client's call-surface redesign, custom hosts use `transport::PreparedRequest` and its immutable
getters. The adapter builds through `RequestBuilder` exact setters and
`advanced::prepare`; SDK JSON tokens never pass through a machine-number projection.
For the native companion, execute the supplied plan with
`dynamic_openapi_client_reqwest::advanced::execute(&client, &request, cancellation)`.
That route applies no configured credential defaults after preparation. Ordinary
native `Client::send` applies configured credentials after eager input setters;
duplicates refuse, and an earlier setter diagnostic takes precedence. The adapter
continues to supply only selected per-attempt credentials before preparation, so
this ordering change does not alter its credential selection or wire headers.
Retaining a callback's owned `PreparedRequest` now retains its operation and source
graph; release it after extracting needed transport facts. A cloned body Arc alone
does not retain that graph.

**SSRF:** server URLs come from the OpenAPI document, and redirect locations from the
server. A host invoking untrusted documents must restrict destinations, applying policy
at every request, including each redirect the adapter follows, which reaches
`Transport::execute` as a new request, and at every network hop after DNS resolution,
including proxy connections. A host refusal at any hop completes the call unsuccessfully.
Declared-server membership and credential-origin grants do not authorize access to a
destination or network range. Destination policy remains a host duty even for an
anonymous operation.

## Authenticated calls

The two private kinds support HTTP bearer tokens and API keys declared in headers.
The application supplies already obtained credentials through the existing interface
context; it owns acquisition, storage, trust, TLS and transport policy. The adapter
never logs in, refreshes, retries, verifies token claims, or caches credentials.

```json
{
  "credentials": { "ServiceToken": "application-supplied-token" },
  "configuration": {
    "local.openapi-3.1.r2.security": {
      "schemes": ["ServiceToken"],
      "origins": { "ServiceToken": ["https://api.example.test"] }
    }
  }
}
```

Pass this value with the existing `operation::Request::with_context` or
`BindingRequest.context`. The constants are `SECURITY_POINT_31` and `SECURITY_POINT_30`;
the latter names `local.openapi-3.0.r2.security`. Ordinary operation inputs
remain the native request. Neither the OBI nor synthesis needs or stores secrets.

`schemes` is an exact set of source-declared names, with no duplicate names or array
ordering requirement, and it is optional: without it the alternative is chosen by the
credentials supplied, as [security selection](#security-selection) describes. `[]`
selects a declared anonymous alternative. All schemes in a chosen alternative apply
together. Unselected credentials are never forwarded. Equivalent duplicate alternatives
share a choice; there is no fallback to another choice after failure.
Named `credentials` is the primary API. Existing `bearerToken`, `apiKey`, and
`apiKeys[name]` inputs work when unambiguous; conflicting applicable values refuse.
API-key strings are opaque: empty strings are preserved, while the client's HTTP
field validation rejects malformed values. An empty bearer token is invalid under
that mechanism. The adapter does not judge whether a server will accept a key.

Each selected credential needs caller-approved `origins`. Scheme/host/effective-port
comparison follows the client; it checks the actual request destination too. Source
URLs and challenge examples are untrusted assertions, never permission or a suggested
grant. The source `location` does not authorize release. Applications must decide which
origins are appropriate independently. Explicit nonmatching grants refuse rather
than prompting to widen access. Custom hosts remain trusted application code.
A followed redirect keeps the credential only on the request's own origin; on another
origin it is removed, as [redirects](#redirects) describe. The adapter declares no
preflight hook, so an advisory preflight makes no prediction and does not prove
authentication readiness.

Whenever the adapter reads the context, it first admits the **whole encoded context**
under `Options.context_bytes` (default 64 KiB), including unrelated fields, and a member
name repeated at any depth refuses before any request. Every invocation that gets as far
as choosing its server reads a supplied context, since `<kind>.server` applies even with
one server, so secured and unsecured operations agree; an unsecured operation still reads
no credentials. Public security metadata and incrementally constructed challenges are
bounded by `result_bytes`.
The generic invocation facade separately applies `max_error_bytes`: an oversized
portable error becomes `ERR_EXECUTION_FAILED`, with zero host entries in this path,
under its existing behavior. No truncated challenge is offered as a usable subset.
These are implementation policies, not requirements imposed on generic interfaces.

Basic, OAuth/OIDC schemes, mTLS, query/cookie keys, nonempty bearer/key security requirement arrays,
and statically unusable credential-header combinations remain unsupported. Any
unsupported effective alternative excludes that operation as a whole in this profile.
This includes case-insensitive collisions, invalid HTTP field names, and keys using
Content-Type on an operation with a body. Other supported operations survive.
Synthesis reports runtime prerequisites under the security point and keeps
AND/OR structure in nonsecret coverage details. Missing runtime credentials are not
schema loss or a reason to exclude an otherwise supported operation.

## Synthesis and scope

Synthesis builds typed parts and commits metadata and the complete operations,
sources and bindings maps in one atomic `Document::edit` batch. A checked result
exposes `Conformant<Document>`; use `as_document()` for exact bytes and consume
`into_inner()` to edit it, then establish conformance again.

Synthesis and inspection name operations alike: each inspection target's
`operationKey` suggestion is the key synthesis gives that operation. Keys are
computed over every operation the source enumerates, including
implementation-unsupported ones, so a change in what this implementation supports
moves no other operation's key, and no key depends on document order. A non-empty
string `operationId` that no other enumerated operation shares (exact comparison)
supplies the key. It is kept exactly when it matches the core key grammar
`^[A-Za-z0-9_][A-Za-z0-9_.-]*$`; otherwise every character outside `[A-Za-z0-9_.-]`
becomes `_`, and `_` is prefixed when the result starts with `.` or `-`. Case never
changes. Without a usable `operationId`, the key is derived: the lowercase method,
then each non-empty segment of the exact Paths key, joined by `_`. Each segment drops
template braces, replaces characters outside `[A-Za-z0-9_.-]` with `_`, collapses runs
of `_` and trims them from both ends; a segment left empty contributes nothing.
`GET /tasks/{id}/comments` gives `get_tasks_id_comments` and `GET /` gives `get`.
Operations that would share a key each take their derived key, `_`, and the first 8
lowercase hexadecimal digits of the 64-bit FNV-1a hash of the UTF-8 string
`"{lowercase method} {exact Paths key}"`. A suffixed key that still collides uses all
16 digits. One that collides even then has no key: synthesis reports it as
implementation-unsupported with `openbindings.openapi.operation-key-unresolved`, and
inspection suggests none. No aliases are generated, and each binding key equals its
operation key. Generated keys are author-owned and may be renamed consistently.

Generated operations and inspection's operation framing carry metadata from the
Operation Object's own fields, never the Path Item's. A non-empty `summary` and
`description` become the `description`, joined by a blank line; either alone is used
as is. `deprecated` is carried only when it is `true`. String `tags` are carried in
order without duplicates, and omitted when none remain. `idempotent` is never set,
and `externalDocs`, examples and extensions are not carried.

Zero sources produce a skeleton; one source is preserved, its content carried unchanged;
composition of multiple sources is currently unsupported. A generated input schema
describes the native request: `path` and `query` objects naming the declared parameters
with their scalar types, `required` lists and `additionalProperties: false`, plus `body`
when a request body is declared. A location member is required when any parameter in it
is required, and `body` when the request body is. The schema describes only the
locations an operation can carry here, so it has no `header` or `cookie` member. Supply a
schema evaluator to the generic operation invoker when invoking those declared contracts.
Source schema constraints and output schemas are not translated: coverage reports
projection loss. Callable targets, unsupported known targets, and unestablished
inventory are distinguished. Strict eligibility is not full schema fidelity.

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
Regression tests cover the 3.0 and 3.1 rules for GET and DELETE request bodies that
[request media](#request-media) states.

The supported profile uses GET, POST, PUT, PATCH and DELETE, declared HTTP(S) servers
with variables and relative URLs, scalar path and query parameters, finite request
bodies in JSON media, finite responses in any media, and supplied references. Forms,
security acquisition, streaming delivery, richer parameter locations, callbacks,
webhook services, ambiguous Path Item merges, and literal non-ASCII path templates
are not implemented. HEAD, OPTIONS and TRACE operations remain outside this
implementation's invocation profile. Percent-encoded Unicode paths preserve their
spelling; the client gap for literal Unicode is tracked separately. Response
declarations, `default` included, never make an operation unsupported, whatever media
or headers they declare. Coverage discloses as projection loss the media and headers a
non-2xx response declares, since none of it reaches the caller, and the headers and
schemas a success or `default` response declares, since headers never become values and
no output schema is projected. The
[CRUD consumer](../../examples/openapi-json-path-consumer/src/crud.rs) exercises both
manually authored and synthesized documents through binding and operation invocation,
including an authenticated PATCH.

`Options` owns immutable client/graph budgets and supplied resources. The adapter
caches one prepared source by kind and content and charges retained metadata, a server
list shared by several operations once. Alternating
sources on one provider re-analyzes each source. A cache miss evicts the previous
entry, then analysis runs outside the cache lock before inserting its bounded result.
Concurrent misses may compute identical results more than once; the last completed
analysis replaces the single entry. A failed analysis retains no new entry.
Warm timings apply to repeated use of
the same source. A provider per source avoids alternating-source cache misses when
that matters to the application. No cache lock is held across host I/O.
Source, result, input and body limits are finite. Logical budgets are not RSS
measurements. Host policy belongs to the application; the qualified reqwest companion
follows no redirects itself (the adapter follows them through it, one call per hop) and
performs no implicit retry or credential acquisition.

## Verification

Run `cargo test --locked --offline` for the full adapter suite, including native HTTP.
The consumer verification entry point is `python3 tools/verify-openapi-json-path.py --wasm`
from the repository root. Archive qualification additionally requires
`--archives --sdk-source <cached-sdk-checkout>` at the pinned revision.

The maintained [external consumer](../../examples/openapi-json-path-consumer) shows
inspection, synthesis, save/reload, explicit evaluator setup, received responses and
followed redirects, and a finite call helper that preserves the actual terminal even if
`write` reports closed admission. Its thin TypeScript probe passes JSON as text through
Wasm. It is qualification code, not a published first-class TypeScript adapter API.

The Wasm probes run in Node, not in a browser or a Worker runtime. Native HTTP and the
Node/Wasm seam qualification do not establish browser or Worker network support,
package publication, or full OpenAPI coverage.
