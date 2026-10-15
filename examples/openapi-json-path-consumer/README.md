# OpenAPI JSON-path experiment consumer

A package consumer with exactly pinned Git SDK dependencies and an exactly pinned
independent OpenAPI client. The qualification tool supplies local archive patches;
they are test plumbing. Pass `--archives --sdk-source <cached-sdk-checkout>` to
qualify extracted packages instead of the pinned source graph.

Run from this repository root with the Rust/Wasm/Node tools available:

```sh
python3 tools/verify-openapi-json-path.py --wasm
```

The full adapter tests run native loopback HTTP. This qualifier runs public consumer tests, serial logical-owner
checks and the TypeScript/Node exact-text Wasm bridge. JSON source and payload text
never passes through JavaScript Number. The Wasm host uses Rc state and an actual
pending JavaScript Promise; cancellation and abandonment drop the owned Rust host
future. Logical counters do not measure JS heap, resident memory or browser networking.

`src/lib.rs` illustrates existing inspection/synthesis/invocation facades. `call`
closes input and observes the authoritative terminal even after closed write admission.
It retains output until success. The panic/assert helpers around it are fixture setup,
not a suggested production error policy. The adapter's package README documents its
private r1 contracts and capability limits. Supply the kind explicitly to `author`
and `source`; the native and Node probes exercise 3.1.2 and 3.0.4 through the same
facades. Source edition and kind are independently checked by the adapter.

`src/crud.rs` adds a six-operation records service. Independently authored OBI
documents and literal exchanges live in `src/crud-fixtures`. Both manual and
synthesized documents are reloaded, then invoked through binding and operation
facades on 3.0.4 and 3.1.2. The same suite runs in the local Node/Wasm probe;
PATCH supplies a caller-owned bearer token. PUT, PATCH and DELETE also run over
native loopback HTTP in the adapter's maintained tests.

`examples/measure.rs` compares CPU work with the pinned client and core authoring,
using equal source/result content for synthesis. Stage rows are descriptive diagnostics;
registered end-to-end budgets are evaluated on paired observations. No measurements
here establish whole-network HTTP parity or production browser/Worker performance.
Benchmark work is stopped; the incremental-auth check remains unresolved under its original budgets.
