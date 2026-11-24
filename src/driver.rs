//! Deterministic round driver: scripted policies submit ordinary World commands.

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use rintawa_sdk::{
    types::{ExtensionId, ExtensionInstanceId},
    world::{ItemId, WorldId},
};
use rintawa_storage::SqliteWorldStorage;
use rintawa_world::WorldCommand;
use rintawa_world_runtime::{WorldRuntime, WorldRuntimeBuilder, WorldSnapshot};

use crate::{
    model::{ActorView, Policy, PolicyDecision, RoundReport, SimulationReport, WorldView},
    schemas::{act_key, actor_key, property_definitions, runtime_schemas, seed_key, state_key},
    system::{ActPayload, ActSystem, ActionWire, SeedActor, SeedPayload, SeedSystem},
};

/// One actor declared by a scenario.
#[derive(Clone)]
pub struct ScenarioActor {
    /// Display name.
    pub name: String,
    /// Initial personal power.
    pub power: i64,
    /// Decision policy, scripted today and model-driven later.
    pub policy: Arc<dyn Policy>,
}

impl std::fmt::Debug for ScenarioActor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScenarioActor")
            .field("name", &self.name)
            .field("power", &self.power)
            .field("policy", &self.policy.name())
            .finish()
    }
}

/// Starting conditions of one run.
#[derive(Debug, Clone)]
pub struct Scenario {
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
}

struct ActorRef {
    id: ItemId,
    name: String,
    policy: Arc<dyn Policy>,
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
            actors: scenario
                .actors
                .iter()
                .zip(&actors)
                .map(|(config, actor)| SeedActor {
                    id: actor.id.to_string(),
                    name: config.name.clone(),
                    power: config.power,
                })
                .collect(),
        })?;
        runtime
            .submit(WorldCommand::system(seed_key(), payload))?
            .wait()?;

        Ok(Self {
            runtime,
            state,
            actors,
            rounds: 0,
            history: Vec::new(),
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
                self.runtime
                    .submit(WorldCommand::system(act_key(), payload))?
                    .wait()?;
            }
            let view = self.view()?;
            self.history.push(RoundReport {
                round: self.rounds,
                stability: view.stability,
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
            history: self.history,
        })
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
            });
        }
        actors.sort_by_key(|actor| actor.id.into_bytes());

        Ok(WorldView {
            round: self.rounds,
            stability,
            ruler,
            actors,
        })
    }
}
