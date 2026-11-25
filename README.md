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

`--format markdown` renders the same report as GitHub-Flavored Markdown tables,
ready to paste into a chat client such as Fluxer; `--format json` (the default)
keeps the machine-readable form. Cell contents are escaped, so a pipe in an actor
name cannot break the table.

Output is a JSON report: world outcome, per-round history with the events that
caused it, and per-actor metrics.

```json
{
  "rounds": 3,
  "final_stability": 4,
  "ruler": "doe",
  "alive": 2,
  "deaths": 1,
  "ruler_changes": 2,
  "history": [
    {
      "round": 1,
      "stability": 6,
      "ruler": "kitten",
      "alive": ["kitten", "doe", "mart"],
      "events": ["kitten took the throne", "doe hoarded power", "mart shared power"]
    }
  ],
  "actors": [
    {
      "name": "kitten", "policy": "aggressive", "alive": true, "power": 4,
      "acts": 3, "grabs": 1, "purges": 1, "kills": ["mart"], "killed_by": null
    }
  ]
}
```

## Rules

The world, not the policies, enforces the rules. `rules: "loose"` is the original
behaviour: any living actor may seize the throne or purge anyone, and power and
stability are only signals. `rules: "strict"` makes them real:

- a coup needs **strictly more** power than the current ruler;
- a purge needs **at least as much** power as its target and spends two power;
- purging the ruler leaves the throne **vacant**;
- at zero stability only `share` is accepted: the realm tolerates no coup, purge or
  hoard, but sharing is the action that restores it;
- each round at zero stability costs **the ruler** one power, because the country
  falls apart under their rule. Challengers are not punished: the ruler's way out is
  to share, and waiting stays viable.

A refused command is a result, not a crash: the driver records it as a `refused`
decision with the world's reason, and the run continues. `examples/strict.json`
plays the strict rules with three glm models and a loyalist bot.

## Templates

A run starts from a scenario, and a scenario is a portable JSON template:

```sh
cargo run --release -- --save-template court.json
cargo run --release -- --template examples/court.json --rounds 6
```

```json
{
  "schema": 1,
  "name": "court",
  "stability": 6,
  "actors": [
    { "name": "ruler", "power": 5, "loyal": false, "policy": "aggressive" },
    { "name": "bot-a", "power": 3, "loyal": true, "policy": "loyalist" },
    { "name": "rival-a", "power": 4, "loyal": false, "policy": "opportunist" }
  ]
}
```

Templates are how a run is branched: the world model has no rewind, so comparing
scenarios means starting a new world from a reviewed starting state, not editing
history. Loading validates the format version, stability and power ranges, actor
counts and policy names, and a test proves that a template reproduces the run it
was saved from. `examples/court.json` is a ready-made court scenario.

## Metrics

The report separates three things that are easy to conflate:

- **world outcome** — stability, current ruler, survivors, how often the throne
  changed hands;
- **round history** — what actually happened, in submission order, so a run can be
  read without replaying it (the world model has no replay);
- **per-actor metrics** — commands submitted, coups, purges, who removed whom and
  who removed the actor.

That last part is the point of a baseline: an LLM policy should be compared on
the same counters, not on how convincing its prose looked.

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

Four scripted implementations ship as the baseline: `Aggressive` takes the throne
when strong enough and purges the weakest rival otherwise, `Cautious` prefers
stability, `Opportunist` hoards power then takes the throne, and `Loyalist`
executes the ruler's will without ever competing for power.

Loyalty is world state, not a policy opinion: every actor carries a `loyal`
boolean in `sim.actor@1`. A loyalist with no ruler abstains, a loyalist with a
ruler removes the strongest actor that is neither the ruler nor another loyalist,
and the other policies treat loyalists as allies rather than purge targets. That
is the "bots following a direct order" case from the chat, and it is checked by a
test that also proves a loyalist never takes the throne.

### Model-driven policy

`LlmPolicy` implements the same trait against an OpenAI-compatible
chat-completions endpoint:

```sh
export RINTAWA_SIM_LLM_ENDPOINT=http://127.0.0.1:8080/v1/chat/completions
export RINTAWA_SIM_LLM_MODEL=some-local-model
export RINTAWA_SIM_LLM_KEY=optional-bearer-token
export RINTAWA_SIM_LLM_BUDGET_TOKENS=20000
cargo run --release -- --rounds 8 --llm
```

Every actor can use a different endpoint, provider or model in the same run: the
`llm` block is per actor, the key is never stored (only the name of the environment
variable that holds it), and the report labels the policy with the model, so
`llm:glm-5.4` and `llm:glm-4.5-air` are distinguishable in the metrics.

```json
{ "name": "big",   "power": 5, "loyal": false, "policy": "llm",
  "llm": { "endpoint": "https://glm.t3ks.com/v1/chat/completions",
           "model": "glm-5.4", "key_env": "ZAI_API_KEY", "budget_tokens": 4000 } },
{ "name": "small", "power": 3, "loyal": false, "policy": "llm",
  "llm": { "endpoint": "https://glm.t3ks.com/v1/chat/completions",
           "model": "glm-4.5-air", "key_env": "ZAI_API_KEY", "budget_tokens": 4000 } },
{ "name": "scripted", "power": 2, "loyal": false, "policy": "opportunist" }
```

`examples/haiku.json` compares `anthropic/claude-haiku-5.5` with `glm-5.3-flash` and a
free model; Haiku is a paid OpenRouter model, so this template only runs once the
OpenRouter key has credits.

`examples/run-mixed-free-6-rounds.md` is a full run rendered for a chat client:
six actors across z.ai and free OpenRouter models, six rounds, with the models'
own reasons for the turning points (~2.2k characters, fits a single message).

`examples/mixed.json` is a ready-made run with three different models plus one
scripted actor, and `examples/mixed-free.json` mixes z.ai and free OpenRouter models
in the same world (six actors, six rounds). `examples/openrouter.json` does the same with free OpenRouter
models:

```sh
export OPENROUTER_API_KEY=...
cargo run --release -- --template examples/openrouter.json --rounds 3 --format markdown
```

Free model identifiers change over time, so check the live list first and keep the
template in sync:

```sh
curl -s https://openrouter.ai/api/v1/models \
  | python3 -c 'import json,sys; print("\n".join(m["id"] for m in json.load(sys.stdin)["data"] if m["id"].endswith(":free")))'
```

Free tiers are rate limited and reasoning-heavy models sometimes answer with an
empty `content`; both surface as `Abstain` rather than as a failed run, so read the
per-actor `acts` count before drawing conclusions about a model. A model that spends
the whole completion budget on thinking returns `finish_reason: length` with `null`
content, so an `llm` block may raise its own budget:

```json
{ "model": "liquid/lfm-3-1.7b:free", "max_tokens": 1500 }
```

## Providers

Any OpenAI-compatible `chat/completions` endpoint works, one per actor. Besides
OpenRouter and z.ai, `examples/bothub.json` targets BotHub
(`https://bothub.chat/api/v2/openai/v1`), which is reachable from networks where
OpenRouter refuses the request; put its key in `BOTHUB_API_KEY` and check the exact
model id with `GET /api/v2/openai/v1/models` before the first run.

## Remote runs

A provider may be unreachable from the machine that owns the world: OpenRouter, for
example, answers `403 Access denied by security policy` from some networks. The
world does not care where the process runs, so build once and run where the provider
is reachable:

```sh
tar czf /tmp/sim-src.tgz --exclude=./target --exclude=./.git .
scp /tmp/sim-src.tgz host:/tmp/ && ssh host 'mkdir -p /tmp/sim && tar xzf /tmp/sim-src.tgz -C /tmp/sim'
ssh host 'cd /tmp/sim && docker run --rm -v /tmp/sim:/src -w /src rust:1-slim-bookworm \
  cargo build --release'
ssh host 'cd /tmp/sim && OPENROUTER_API_KEY=... ./target/release/rintawa-sim \
  --template examples/openrouter.json --rounds 3 --format markdown'
```

`RINTAWA_SIM_LLM_PROXY=socks5://127.0.0.1:1080` is honoured when the provider is
reachable only through a tunnel. Only OpenAI-compatible `chat/completions` is required, so a local
server, a hosted provider or a self-hosted gateway can share a world.

Every action carries the policy's explanation. Model-driven policies ask for a
one-line `reason` in the same JSON object and fall back to the provider's
`reasoning`/`reasoning_content` text when the model omits it, so a run answers "why
did it do that" without re-running anything:

| актор | раунд | действие | почему |
|---|---:|---|---|
| nemotron | 1 | `hoard` | Consolidate power base before challenging stronger rival |

`RINTAWA_SIM_LLM_TRACE=1` prints each decision to stderr while the report keeps
only the effects. Reasoning models spend completion tokens on thinking first, so
the request asks for 512 completion tokens; with a smaller budget such a model can
return an empty `content`, which the policy treats as `Abstain` (a real run against
`glm-5.3-flash` did exactly that in one round).

The policy is deliberately defensive, because an actor in a world must not stall
the world: a transport error, a non-JSON answer, an unknown target, a purge
without a live target or an exhausted token budget all end in `Abstain`, which the
driver simply skips. The response is parsed, not trusted: the action name is
validated and the target must be an alive rival from the view the model was given.
Tests drive it against a mock HTTP server, so the model path is exercised without
any network access or API key.

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
