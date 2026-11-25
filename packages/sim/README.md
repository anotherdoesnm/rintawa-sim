# Simulation package (skeleton)

An RTW package that would run the simulation inside a World instead of next to it.

## What is already here

- `rtwkit.toml`, `rtw/rtw.toml`, `rtw/manifest.toml` — package identity, artifact
  layout and the three runtime permissions the simulation needs.
- `src/wasm.rs` — the boundary: why it does not compile yet, and what it will do.
- `build.sh` — assembles the artifact root and refuses to produce a `.rtw` without a
  backend component.

## What is missing, and why

1. **Engine WIT.** The guest must be generated against `wit/engine.wit` from the
   rtwKit branch this package targets (`feat/mvp-ai-narrator`, world
   `task-runtime-plugin`). Vendoring a copy here would drift silently.
2. **Exported entry points.** The world-system and service-protocol names are defined
   by that WIT; the adapter cannot invent them.
3. **Inference authority.** Only the World's selected Narrator may start inference, so
   the coordinator asks the Narrator (or the AI Provider once per-request connection
   selection exists) and the package only submits `sim.act@1` commands.
4. **Admission for a non-narrator package.** If the owner prefers the simulation to be
   its own package instead of a component inside the Narrator artifact, the admission
   rule in `ai-provider` needs a second allowed consumer; that is an rtwKit decision,
   not a local one.

## What is reused without changes

`rintawa-sim` provides the property definitions (`sim.state@1`, `sim.actor@1`), the
command schemas (`sim.seed@1`, `sim.act@1`), both Systems, the rule set, the policies
and the report model. The package only wires them to the component model.
