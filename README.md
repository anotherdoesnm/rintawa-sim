# rintawa-sim

Scripted baseline for **agent simulations** on authoritative Rintawa worlds.

This is a laboratory crate, not a Core feature: it declares ordinary property
definitions and command schemas, evaluates them with ordinary World Systems and
drives a real single-writer world runtime. Nothing here adds tables, jobs, a clock
or replay to the world model.

## What it proves

A narrative simulation with partial information does not need new Core storage. It
needs:

- items for actors and world state (`sim.actor@1`, `sim.state@1`);
- commands (`sim.seed@1`, `sim.act@1`) evaluated by ordinary Systems;
- one transaction per action, committed atomically by the single-writer runtime;
- a policy layer outside the world that decides what an actor does next.

That is exactly the four-table model: objects, relations (the throne is a relation
from the current ruler to the state item, so every coup leaves history behind) and
versioned properties.

## Run

```sh
cargo run --release -- --rounds 8 --db /tmp/world.sqlite
```

Output is a JSON report: stability per round, current ruler, survivors and deaths.

```
{
  "rounds": 6,
  "final_stability": 4,
  "ruler": "doe",
  "alive": 2,
  "deaths": 1,
  "history": [...]
}
```

## Policies

An actor's decisions come from a `Policy` trait:

```rust
pub trait Policy: Send + Sync {
    fn name(&self) -> &str;
    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision;
}
```

`PolicyDecision::Act(..)` submits one ordinary World command for this round;
`PolicyDecision::Abstain` skips the turn, which is how a policy expresses "out of
budget", "unsure" or "waiting for information". The world never sees the policy —
the driver reads a consistent view, asks, and commits the result.

Three scripted implementations ship as the baseline: `Aggressive` takes the throne
when strong enough and purges the weakest rival otherwise, `Cautious` prefers
stability, `Opportunist` hoards power then takes the throne.

The baseline is deterministic: the same scenario produces the same report, which
is checked by a test. That is the point — an LLM policy can only be judged against
a reproducible scripted run, and it plugs into the same trait without touching the
world, the schema or the systems. The policy seam is covered by tests that add a
pacifist policy and an abstaining one from outside the crate.

## Status and limits

- Scripted policies only. An LLM policy is the next step and belongs behind the
  same `Policy` decision interface.
- Not packaged as an RTW artifact yet. `rtwKit` still pins a pre-four-table
  `rintawa-sdk`, so the same scenario has to wait for the package port before it
  can ship as a `.rtw`.
- Pinned to `rintawa` `feat/mvp-runtime-infrastructure` at
  `1e5b82e1c57462067942ed6dc8c396f82a785589`.
- No replay and no rewind by design: a run is one-way, exactly like the world
  model. Branching a scenario means seeding a new world, not rewinding this one.
