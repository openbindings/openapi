# OpenAPI JSON-path consumer

A package consumer with exactly pinned Git SDK dependencies and an exactly pinned
independent OpenAPI client. The qualification tool supplies local archive patches;
they are test plumbing. Pass `--archives --sdk-source <cached-sdk-checkout>` to
qualify extracted packages instead of the pinned source graph.

Run from this repository root with the Rust/Wasm/Node tools available:

```sh
python3 tools/verify-openapi-json-path.py --wasm
```

The consumer follows exact JSON through the adapter's public facades: descriptions,
native requests and response values keep their numbers exactly, natively and through
the TypeScript/Node Wasm bridge, where JSON never passes through a JavaScript Number.
The full adapter tests run native loopback HTTP. This qualifier runs the public consumer
tests, serial logical-owner checks and the Node/Wasm probe. The Wasm host uses Rc state
and an actual pending JavaScript Promise; cancellation and abandonment drop the owned
Rust host future. Logical counters do not measure JS heap, resident memory or browser
networking.

`src/lib.rs` illustrates the inspection, synthesis and invocation facades. Supply the
kind explicitly to `author` and `source`; the native and Node probes exercise 3.1.2 and
3.0.4 through the same facades, and the adapter checks source edition and kind
independently. `author` embeds the description in its source as a JSON object, spliced
in as text so its numbers stay exact; `source` carries it as JSON text. `call` sends one
native request, closes input and observes the authoritative terminal even after closed
write admission. It retains output until success; a successful call emits at most one
value. The panic/assert helpers around it are fixture setup, not a suggested production
error policy. The adapter's package README documents its private kinds and capability
limits.

`src/crud.rs` adds a six-operation records service. Independently authored OBI documents,
which carry the description as JSON text, and literal exchanges live in
`src/crud-fixtures`. Both manual and synthesized documents are reloaded, then invoked
through binding and operation facades on 3.0.4 and 3.1.2; the empty 202 and 204
responses emit no value. PATCH supplies a caller-owned bearer token. PUT, PATCH and
DELETE also run over native loopback HTTP in the adapter's maintained tests.

`src/responses.rs` sends the same hand-authored records documents responses their
description does not declare: text becomes a string, other media and a missing
`Content-Type` become Base64, a repeated JSON member keeps its last occurrence with its
exact number, and empty content emits no value. It follows a 302, a 303 that turns a
POST into a GET without content, and a 307 to another origin that repeats the PATCH
without its bearer token, each hop a separate host call. A redirect without `Location`
and an https to http redirect are final responses and fail without data.

`src/auth.rs` and `src/auth_acceptance.rs` cover authenticated calls: security
alternatives chosen by the credentials supplied or by `schemes`, caller-approved origins,
challenges and deliberate new attempts, server choice with a `url`, references resolved
against the source `location`, and context bounds. A 401, a 403 and a 302 without
`Location` complete unsuccessfully with no data.

The CRUD, response and authentication suites also run in the Node/Wasm probe, so the
adapter's request, response and redirect code is exercised in the Wasm build.

`examples/measure.rs` compares CPU work with the pinned client and core authoring,
using equal source/result content for synthesis. Stage rows are descriptive diagnostics;
registered end-to-end budgets are evaluated on paired observations. No measurements
here establish whole-network HTTP parity or production browser/Worker performance.
Benchmark work is stopped; the incremental-auth check remains unresolved under its original budgets.
