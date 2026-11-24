# Integration with the Rintawa core (variant B: the Narrator drives the agents)

Observations taken from `martrtr/rtwKit` branch `feat/mvp-ai-narrator` (`4b66f41a`),
which pins `rintawa@b997e98e` — a commit that already contains the four-table World
model this repository targets.

## How inference is admitted

Inference is not a Core WIT interface. It is a package service:

- contract `rintawa.inference.text@1`, point `rintawa/inference/v1`;
- typed by `crates/inference-contracts` (`InferenceMessage`, `InferenceRole`);
- implemented by the `ai-provider` package, which also owns the shared catalog of
  connections (published preference `rintawa.ai.connections.v1`) and the per-world
  secret read approval (`ai.connection_<hex>.api_key`).

`packages/ai-provider/src/wasm.rs` decides who may generate:

```rust
Call::BeginGenerate { messages } => {
    if context.world_id.is_none()
        || !selected("rintawa.narrator").is_some_and(|n| same(&n, p))
    {
        return Err("access-denied");
    }
    ...
}
```

So the admission rule is: the caller principal must be **the instance that the
composition selected for package id `rintawa.narrator`**, inside a World. A separate
package, whatever its own id, is refused with `access-denied`.

## What that means for rintawa-sim

"Variant B" (the Narrator drives the agents, the simulation never talks to a model)
cannot be built as an unrelated package that exposes a service to the narrator:
`rintawa.narrator` is a concrete chat-oriented package with `rintawa/narrator/v1`
(`Generate { conversationId, userMessageId, messageId, entryId }`) and no hook for
"ask an external package what an agent does next".

The workable shape is therefore:

1. Ship the simulation **inside the narrator identity**: a package that declares
   `replaces = ["rintawa.narrator"]`, speaks `rintawa/narrator/v1` to Chat, and
   additionally evaluates the simulation Systems. The admission check passes because
   the selected instance for `rintawa.narrator` is ours.
2. The Narrator component is the only one that calls
   `rintawa/inference/v1`; the simulation asks it for a decision through an internal
   contract (a service point of the same package, e.g. `rintawa/sim/v1`), so keys,
   connection choice and HTTP approvals stay with the AI package and the Host.
3. Systems and schemas move into the package: property definitions through
   `world-registration`, command handling through the World System service protocol,
   the observer view through a projection.

## What stays out of the package

- The world itself: one SQLite file, four tables, single writer (unchanged).
- The driver: a World has no clock, so something must submit commands. Inside the
  package that is a `background-task`; outside it is the lab driver this repository
  already has, which is also how the current runs are produced.
- The report: an observer projection plus the CLI renderer.

## Consequence for this repository

The current crate stays the reference implementation and the measurement harness
(deterministic baseline, per-actor metrics, reasons, comparisons). The package is a
second artifact that reuses `model.rs`, `schemas.rs` and `system.rs` and replaces only
the transport (`Policy` becomes a service call inside the package) and the driver
(background task instead of the CLI loop).
