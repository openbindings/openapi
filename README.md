The OpenBindings implementation of the source-inspection, interface-synthesis and binding-invocation interfaces for OpenAPI, built on [`openapi-client`](https://github.com/openbindings/openapi-client).

The packages are unpublished. This repository depends on the SDK at
`3546fc29d3fb329656c6a7506f6006b9eb38375d` and the independent client at
`c6a8236edbd02f4527d724bab60560861bf2722d`. The SDK defines the interfaces and supplies
no kind-specific instances. This project supplies the OpenAPI provider.

Use `OpenApi::default()` with the SDK's checked `Inspector` or `Synthesizer`.
Use `OpenApi::with_transport` with application-owned execution and the checked
binding or operation invoker. The [JSON-path consumer](examples/openapi-json-path-consumer/)
shows inspection, synthesis, save/reload and invocation with exact JSON, including
authenticated calls, received responses and followed redirects. The independent
[inspection consumer](examples/openapi-inspection-consumer/) covers manual authoring
from either document carrier and supplied-resource recovery, and the
[minimal consumer](examples/openapi-invocation-only/) guards dependency isolation.

The adapter implements the private `local.openapi-3.1.r2` and `local.openapi-3.0.r2`
kinds; the earlier `local.openapi-json-probe.r1` and `local.openapi30-json-probe.r1`
labels are retired and refused. Sources carry the entry `document`, embedded as a JSON
object or as JSON text, and an optional `location`; bindings carry `path` and lowercase
`method`, and an operation's input is the native request. Reloaded documents
must pass SDK validation before operation invocation or
value-contract preparation. The current SDK pin uses one exact `Document`, `Conformant<Document>` proof with infallible typed
views, value contracts handed out by conformant operations and companion-owned
`CancellationToken` interface signatures. Value checks report the verdict alone unless
the caller asks for an explanation; [migration details](PROVENANCE.md) describe the
diagnostic and reference boundaries. It implements no
published OpenAPI binding standard.

The finite profile admits GET, POST, PUT, PATCH and DELETE for OpenAPI 3.0 and 3.1,
supplied references, declared servers with variables, scalar path/query parameters,
finite JSON request bodies, finite responses read by their received media, followed
redirects, and caller-supplied bearer and header API-key credentials.
No schema-constraint translation, security acquisition, implicit fetching, automatic
retry or full OpenAPI coverage is claimed. YAML documents and bindings carrying JSONata
adaptation members are refused for now. HEAD/OPTIONS/TRACE, richer encodings, streaming
delivery and literal Unicode path templates remain outside the invocation profile. See the
[complete package guide](crates/openbindings-openapi/README.md) for the kind contract,
what is refused for now, budgets, security policy and synthesis loss.

The incremental-auth performance check remains unresolved and deferred with
its original budgets unchanged. The stopped measurement examples are compiled and
linted but never run by CI. This repository carries no historical loop evidence.
The Wasm probes run in Node, not in a browser or a Worker runtime: native HTTP and
local Node/Wasm probes do not establish browser/Worker network support or a released
TypeScript interface API.

Use Rust 1.99.0 with the wasm32-unknown-unknown target, wasm-bindgen-cli 0.2.129,
Node 22.19.0, npm and Python 3.13+. With dependency caches populated:

```sh
cargo build --locked --offline
cargo test --locked --offline
npm ci --offline --ignore-scripts
python3 tools/attribution.py --check
python3 tools/verify-openapi-inspection.py --wasm
python3 tools/verify-openapi-json-path.py --wasm
python3 tools/verify-openapi-invocation-only.py
```

The verifiers generate independent consumers under `target/`, preserve selected
lockfile versions, and check a single source identity for each SDK package and the
exact client revision. They run each consumer's tests and lints, check that core gains
no OpenAPI dependency and that the minimal consumer needs no evaluator, reqwest, Tokio
or Wasm runtime, and with `--wasm` run the consumer's Node/Wasm probe. Each writes a
receipt with the hashes of the inputs it tested, the commit checked out and its
commands; with `--wasm` it also keeps the probe's summaries, such as the kinds the
JSON-path probe exercised. They require no SDK checkout in the source-Git mode.
Archive qualification packages only an explicitly supplied clean cached SDK source:

```sh
python3 tools/verify-openapi-json-path.py --archives --wasm --sdk-source <cached-sdk-checkout>
python3 tools/verify-openapi-inspection.py --archives --wasm --sdk-source <cached-sdk-checkout>
python3 tools/verify-openapi-invocation-only.py --archives
```

The SDK source must have the pinned tree, unchanged tracked files and no tracked
symlinks. The pinned revision, checkout commit and tree identity are recorded in the generated receipt.
Extracted SDK packages are patched for both their original Git dependencies and
normalized registry dependencies; no source checkout may escape into the archive
consumer graph. The minimal archive consumer checks the preceding JSON-path
receipt's input and extracted-package hashes before using that snapshot.

CI requires full adapter tests on Linux, macOS and Windows, the independent
consumers, extracted Cargo archives, and both Node/Wasm probes. CI also runs
`cargo deny check advisories`; vulnerabilities and yanked versions fail, and every
advisory ignore requires an ID and a written reason in `deny.toml`. The fixture-specific
`-text` attribute preserves literal CRUD exchanges on every host.
Regenerate `DEPENDENCIES.json` and `THIRD-PARTY-NOTICES.txt` after dependency changes
with `python3 tools/attribution.py`; CI checks both against the offline locked
Cargo graph and installed pinned TypeScript sources. Their `-text` attributes
preserve the generated bytes on every host. Qualification consumers alone enable
the client's non-default `qualification` feature for owner counters.
[PROVENANCE.md](PROVENANCE.md) maps the clean import to SDK source paths.
