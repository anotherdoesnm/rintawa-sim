//! Deterministic round driver: scripted policies submit ordinary World commands.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use rintawa_sdk::{
    types::{ExtensionId, ExtensionInstanceId},
    world::{ItemId, WorldId},
};
use rintawa_storage::SqliteWorldStorage;
use rintawa_world::WorldCommand;
use rintawa_world_runtime::{
    SystemError, WorldRuntime, WorldRuntimeBuilder, WorldRuntimeError, WorldSnapshot,
};

use crate::{
    model::{
        ActorReport, ActorView, Policy, PolicyDecision, RoundReport, Rules, SimulationReport,
        WorldView,
    },
    schemas::{
        act_key, actor_key, property_definitions, runtime_schemas, seed_key, state_key, tick_key,
    },
    system::{ActPayload, ActSystem, ActionWire, SeedActor, SeedPayload, SeedSystem, TickSystem},
};

/// One actor declared by a scenario.
#[derive(Clone)]
pub struct ScenarioActor {
    /// Display name.
    pub name: String,
    /// Initial personal power.
    pub power: i64,
    /// Whether the actor executes orders instead of competing for power.
    pub loyal: bool,
    /// Decision policy, scripted or model-driven.
    pub policy: Arc<dyn Policy>,
    /// Endpoint configuration when the policy is model-driven, kept so a scenario
    /// can be saved back to a template.
    pub llm: Option<crate::llm::LlmConfig>,
}

impl std::fmt::Debug for ScenarioActor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScenarioActor")
            .field("name", &self.name)
            .field("power", &self.power)
            .field("loyal", &self.loyal)
            .field("policy", &self.policy.name())
            .field("llm", &self.llm)
            .finish()
    }
}

/// Starting conditions of one run.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// Rule set the world enforces.
    pub rules: Rules,
    /// Initial stability.
    pub stability: i64,
    /// Actors present at the start.
    pub actors: Vec<ScenarioActor>,
}

/// Owner identity recorded on the package schemas.
pub fn package_owner() -> ExtensionId {
    ExtensionId::new("rintawa.sim.lab")
}

/// Runs one scripted scenario against a real single-writer world runtime.
pub struct SimDriver {
    runtime: WorldRuntime,
    state: ItemId,
    actors: Vec<ActorRef>,
    rounds: u32,
    history: Vec<RoundReport>,
    rules: Rules,
    stats: Vec<ActorStats>,
    ruler_changes: u32,
}

struct ActorRef {
    id: ItemId,
    name: String,
    policy: Arc<dyn Policy>,
}

/// Running per-actor counters collected while commands are submitted.
#[derive(Default)]
struct ActorStats {
    acts: u32,
    grabs: u32,
    purges: u32,
    kills: Vec<String>,
    killed_by: Option<String>,
    decisions: Vec<crate::model::DecisionReport>,
}

impl SimDriver {
    /// Creates the world, registers the package schemas and seeds the scenario.
    ///
    /// # Errors
    ///
    /// Returns a storage, runtime or validation error.
    pub fn start(db_path: &Path, scenario: &Scenario) -> Result<Self> {
        if db_path.exists() {
            std::fs::remove_file(db_path)
                .with_context(|| format!("cannot reset {}", db_path.display()))?;
        }
        let storage = SqliteWorldStorage::create(db_path, WorldId::new())?;
        let mut builder = WorldRuntimeBuilder::new(storage);
        let owner = ExtensionInstanceId::new("sim.baseline");
        for definition in runtime_schemas(&package_owner()) {
            builder.register_runtime_schema(owner.clone(), definition)?;
        }
        builder.register_system(SeedSystem::new())?;
        builder.register_system(ActSystem::new())?;
        builder.register_system(TickSystem::new())?;
        let runtime = builder.start()?;
        let definitions: Vec<_> = property_definitions(&package_owner())
            .into_iter()
            .map(|definition| (owner.clone(), definition))
            .collect();
        runtime.register_property_definitions(&definitions)?;

        let state = ItemId::new();
        let actors: Vec<ActorRef> = scenario
            .actors
            .iter()
            .map(|actor| ActorRef {
                id: ItemId::new(),
                name: actor.name.clone(),
                policy: Arc::clone(&actor.policy),
            })
            .collect();

        let payload = serde_json::to_value(SeedPayload {
            state: state.to_string(),
            stability: scenario.stability,
            rules: scenario.rules,
            actors: scenario
                .actors
                .iter()
                .zip(&actors)
                .map(|(config, actor)| SeedActor {
                    id: actor.id.to_string(),
                    name: config.name.clone(),
                    power: config.power,
                    loyal: config.loyal,
                })
                .collect(),
        })?;
        runtime
            .submit(WorldCommand::system(seed_key(), payload))?
            .wait()?;

        let stats = actors.iter().map(|_| ActorStats::default()).collect();
        Ok(Self {
            runtime,
            state,
            actors,
            rounds: 0,
            history: Vec::new(),
            rules: scenario.rules,
            stats,
            ruler_changes: 0,
        })
    }

    /// Plays the requested number of rounds and records a report per round.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when a command is rejected or the world cannot be read.
    pub fn run(&mut self, rounds: u32) -> Result<()> {
        for _ in 0..rounds {
            self.rounds += 1;
            let ruler_before = self.view()?.ruler;
            let mut events = Vec::new();
            let order: Vec<ItemId> = {
                let view = self.view()?;
                view.actors
                    .iter()
                    .filter(|actor| actor.alive)
                    .map(|actor| actor.id)
                    .collect()
            };
            for actor_id in order {
                let view = self.view()?;
                let Some(me) = view
                    .actors
                    .iter()
                    .find(|actor| actor.id == actor_id)
                    .cloned()
                else {
                    continue;
                };
                if !me.alive {
                    continue;
                }
                let Some(actor) = self.actors.iter().find(|actor| actor.id == actor_id) else {
                    continue;
                };
                let planned = match actor.policy.decide(&view, &me) {
                    PolicyDecision::Act(planned) => planned,
                    PolicyDecision::Abstain => continue,
                };
                let payload = serde_json::to_value(ActPayload {
                    state: self.state.to_string(),
                    actor: actor_id.to_string(),
                    action: match planned.kind {
                        crate::model::ActionKind::GrabPower => ActionWire::GrabPower,
                        crate::model::ActionKind::Purge => ActionWire::Purge,
                        crate::model::ActionKind::Share => ActionWire::Share,
                        crate::model::ActionKind::Hoard => ActionWire::Hoard,
                    },
                    target: planned.target.map(|target| target.to_string()),
                })?;
                let outcome = self
                    .runtime
                    .submit(WorldCommand::system(act_key(), payload))?
                    .wait();
                let index = self.actor_index(actor_id);
                if let Err(error) = outcome {
                    if let Some(reason) = rejection(&error) {
                        let name = self.actors[index].name.clone();
                        let refused_target = planned
                            .target
                            .map(|t| self.actors[self.actor_index(t)].name.clone());
                        events.push(format!("{name} refused: {reason}"));
                        self.stats[index]
                            .decisions
                            .push(crate::model::DecisionReport {
                                round: self.rounds,
                                action: String::from("refused"),
                                target: refused_target,
                                note: Some(reason),
                            });
                        continue;
                    }
                    return Err(error.into());
                }
                let name = self.actors[index].name.clone();
                self.stats[index].acts += 1;
                let decision_target = planned
                    .target
                    .map(|target| self.actors[self.actor_index(target)].name.clone());
                self.stats[index]
                    .decisions
                    .push(crate::model::DecisionReport {
                        round: self.rounds,
                        action: match planned.kind {
                            crate::model::ActionKind::GrabPower => String::from("grab-power"),
                            crate::model::ActionKind::Purge => String::from("purge"),
                            crate::model::ActionKind::Share => String::from("share"),
                            crate::model::ActionKind::Hoard => String::from("hoard"),
                        },
                        target: decision_target,
                        note: planned.note.clone(),
                    });
                match planned.kind {
                    crate::model::ActionKind::GrabPower => {
                        self.stats[index].grabs += 1;
                        events.push(format!("{name} took the throne"));
                    }
                    crate::model::ActionKind::Purge => {
                        self.stats[index].purges += 1;
                        if let Some(target) = planned.target {
                            let target_index = self.actor_index(target);
                            let target_name = self.actors[target_index].name.clone();
                            self.stats[index].kills.push(target_name.clone());
                            self.stats[target_index].killed_by = Some(name.clone());
                            events.push(format!("{name} purged {target_name}"));
                        }
                    }
                    crate::model::ActionKind::Share => events.push(format!("{name} shared power")),
                    crate::model::ActionKind::Hoard => events.push(format!("{name} hoarded power")),
                }
            }
            let tick = serde_json::json!({ "state": self.state.to_string() });
            self.runtime
                .submit(WorldCommand::system(tick_key(), tick))?
                .wait()?;
            let view = self.view()?;
            if view.ruler.is_some() && view.ruler != ruler_before {
                self.ruler_changes += 1;
            }
            self.history.push(RoundReport {
                round: self.rounds,
                stability: view.stability,
                events,
                ruler: view
                    .ruler
                    .and_then(|ruler| view.actors.iter().find(|actor| actor.id == ruler))
                    .map(|actor| actor.name.clone()),
                alive: view
                    .actors
                    .iter()
                    .filter(|actor| actor.alive)
                    .map(|actor| actor.name.clone())
                    .collect(),
            });
        }
        Ok(())
    }

    /// Consumes the driver and returns the run report.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the final world state cannot be read.
    pub fn finish(self) -> Result<SimulationReport> {
        let view = self.view()?;
        let mut actors: Vec<ActorReport> = view
            .actors
            .iter()
            .map(|actor| {
                let index = self.actor_index(actor.id);
                let stats = &self.stats[index];
                ActorReport {
                    name: actor.name.clone(),
                    policy: self.actors[index].policy.name().to_string(),
                    alive: actor.alive,
                    power: actor.power,
                    acts: stats.acts,
                    grabs: stats.grabs,
                    purges: stats.purges,
                    kills: stats.kills.clone(),
                    killed_by: stats.killed_by.clone(),
                    decisions: stats.decisions.clone(),
                }
            })
            .collect();
        actors.sort_by(|left, right| left.name.cmp(&right.name));
        let ruler = view
            .ruler
            .and_then(|ruler| view.actors.iter().find(|actor| actor.id == ruler))
            .map(|actor| actor.name.clone());
        Ok(SimulationReport {
            rounds: self.rounds,
            final_stability: view.stability,
            ruler,
            alive: view.alive(),
            deaths: u32::try_from(view.actors.len().saturating_sub(view.alive()))
                .unwrap_or_default(),
            rules: self.rules.name().to_string(),
            ruler_changes: self.ruler_changes,
            history: self.history,
            actors,
        })
    }

    fn actor_index(&self, id: ItemId) -> usize {
        self.actors
            .iter()
            .position(|actor| actor.id == id)
            .unwrap_or_default()
    }

    /// Reads one consistent view of the simulated world.
    fn view(&self) -> Result<WorldView> {
        let snapshot: WorldSnapshot = self.runtime.snapshot()?;
        let state_raw = snapshot
            .load_property(self.state, &state_key())
            .map_err(|error| anyhow::anyhow!("state read failed: {error}"))?
            .map(|record| record.data().clone())
            .context("world state property is missing")?;
        let stability = state_raw
            .get("stability")
            .and_then(serde_json::Value::as_i64)
            .context("world state has no stability")?;
        let rules = state_raw
            .get("rules")
            .and_then(serde_json::Value::as_str)
            .map(|name| {
                if name == "strict" {
                    Rules::Strict
                } else {
                    Rules::Loose
                }
            })
            .unwrap_or(self.rules);
        let ruler = state_raw
            .get("ruler")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<ItemId>().ok());

        let mut actors = Vec::with_capacity(self.actors.len());
        for actor in &self.actors {
            let raw = snapshot
                .load_property(actor.id, &actor_key())
                .map_err(|error| anyhow::anyhow!("actor read failed: {error}"))?
                .map(|record| record.data().clone())
                .with_context(|| format!("actor {} has no property", actor.name))?;
            actors.push(ActorView {
                id: actor.id,
                name: actor.name.clone(),
                power: raw
                    .get("power")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
                alive: raw
                    .get("alive")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                is_ruler: ruler == Some(actor.id),
                loyal: raw
                    .get("loyal")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            });
        }
        actors.sort_by_key(|actor| actor.id.into_bytes());

        let history: Vec<RoundReport> = self
            .history
            .iter()
            .rev()
            .take(3)
            .rev()
            .cloned()
            .collect();
        Ok(WorldView {
            round: self.rounds,
            history,
            rules,
            stability,
            ruler,
            actors,
        })
    }
}

/// Returns the world's rejection reason when the command was refused by rules.
fn rejection(error: &WorldRuntimeError) -> Option<String> {
    match error {
        WorldRuntimeError::SystemFailed { source, .. } => match source {
            SystemError::Rejected(reason) => Some(reason.clone()),
            _ => None,
        },
        _ => None,
    }
}
