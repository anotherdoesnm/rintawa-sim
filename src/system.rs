//! World Systems that evaluate the simulation commands into item/property changes.

use rintawa_sdk::world::{ItemId, SchemaKey};
use rintawa_world::{StateChange, WorldTransaction};
use rintawa_world_runtime::{SystemError, SystemResult, WorldSnapshot, WorldSystem};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::schemas::{act_key, actor_key, seed_key, state_key};

/// Payload of the seed command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedPayload {
    /// Identity of the world-state item.
    pub state: String,
    /// Initial stability.
    pub stability: i64,
    /// Actors created at the start of the run.
    pub actors: Vec<SeedActor>,
}

/// One actor declared by the seed command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedActor {
    /// Identity of the actor item.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Initial personal power.
    pub power: i64,
}

/// Payload of one act command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActPayload {
    /// Identity of the world-state item.
    pub state: String,
    /// Identity of the acting item.
    pub actor: String,
    /// Action kind as it appears on the wire.
    pub action: ActionWire,
    /// Target of a purge.
    #[serde(default)]
    pub target: Option<String>,
}

/// Wire form of one action kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionWire {
    /// Take the throne.
    GrabPower,
    /// Remove one other actor.
    Purge,
    /// Spend power to raise stability.
    Share,
    /// Convert stability into power.
    Hoard,
}

/// State property payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateData {
    stability: i64,
    ruler: Option<String>,
}

/// Actor property payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActorData {
    name: String,
    power: i64,
    alive: bool,
}

fn parse_item(value: &str, field: &str) -> SystemResult<ItemId> {
    value
        .parse::<ItemId>()
        .map_err(|_| SystemError::Rejected(format!("{field} is not a valid item identity")))
}

fn read_property(
    snapshot: &WorldSnapshot,
    item: ItemId,
    key: &SchemaKey,
) -> SystemResult<Option<Value>> {
    snapshot
        .load_property(item, key)
        .map(|record| record.map(|record| record.data().clone()))
        .map_err(|error| SystemError::Failed(format!("snapshot read failed: {error}")))
}

/// System that creates the initial actors and world state.
pub struct SeedSystem {
    schema: SchemaKey,
}

impl SeedSystem {
    /// Creates the seed System for the package seed schema.
    pub fn new() -> Self {
        Self { schema: seed_key() }
    }
}

impl Default for SeedSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldSystem for SeedSystem {
    fn command_schema(&self) -> &SchemaKey {
        &self.schema
    }

    fn evaluate(
        &self,
        _snapshot: &WorldSnapshot,
        command: &rintawa_world::WorldCommand,
    ) -> SystemResult<WorldTransaction> {
        let payload: SeedPayload = serde_json::from_value(command.payload().clone())
            .map_err(|error| SystemError::Rejected(format!("invalid seed payload: {error}")))?;
        let state = parse_item(&payload.state, "state")?;

        let mut transaction = WorldTransaction::new();
        transaction.push_change(StateChange::CreateObject { id: state });
        transaction.push_change(StateChange::SetProperty {
            item: state,
            key: state_key(),
            data: json!({ "stability": payload.stability, "ruler": Value::Null }),
        });

        for actor in &payload.actors {
            let id = parse_item(&actor.id, "actor")?;
            transaction.push_change(StateChange::CreateObject { id });
            transaction.push_change(StateChange::SetProperty {
                item: id,
                key: actor_key(),
                data: json!({ "name": actor.name, "power": actor.power, "alive": true }),
            });
        }
        Ok(transaction)
    }
}

/// System that resolves one actor action against the current world state.
pub struct ActSystem {
    schema: SchemaKey,
}

impl ActSystem {
    /// Creates the act System for the package act schema.
    pub fn new() -> Self {
        Self { schema: act_key() }
    }
}

impl Default for ActSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldSystem for ActSystem {
    fn command_schema(&self) -> &SchemaKey {
        &self.schema
    }

    fn evaluate(
        &self,
        snapshot: &WorldSnapshot,
        command: &rintawa_world::WorldCommand,
    ) -> SystemResult<WorldTransaction> {
        let payload: ActPayload = serde_json::from_value(command.payload().clone())
            .map_err(|error| SystemError::Rejected(format!("invalid act payload: {error}")))?;
        let state = parse_item(&payload.state, "state")?;
        let actor = parse_item(&payload.actor, "actor")?;

        let state_raw = read_property(snapshot, state, &state_key())?
            .ok_or_else(|| SystemError::Rejected(String::from("world state is missing")))?;
        let mut state_data: StateData = serde_json::from_value(state_raw)
            .map_err(|error| SystemError::Failed(format!("corrupt state payload: {error}")))?;

        let actor_raw = read_property(snapshot, actor, &actor_key())?
            .ok_or_else(|| SystemError::Rejected(String::from("acting item is not an actor")))?;
        let mut actor_data: ActorData = serde_json::from_value(actor_raw)
            .map_err(|error| SystemError::Failed(format!("corrupt actor payload: {error}")))?;
        if !actor_data.alive {
            return Err(SystemError::Rejected(String::from(
                "dead actors cannot act",
            )));
        }

        let mut transaction = WorldTransaction::new();
        match payload.action {
            ActionWire::GrabPower => {
                state_data.ruler = Some(actor.to_string());
                transaction.push_change(StateChange::CreateRelation {
                    id: ItemId::new(),
                    subject: actor,
                    object: state,
                });
            }
            ActionWire::Purge => {
                let target = payload
                    .target
                    .as_deref()
                    .ok_or_else(|| SystemError::Rejected(String::from("purge needs a target")))?;
                let target = parse_item(target, "target")?;
                if target == actor {
                    return Err(SystemError::Rejected(String::from(
                        "an actor cannot purge itself",
                    )));
                }
                let target_raw =
                    read_property(snapshot, target, &actor_key())?.ok_or_else(|| {
                        SystemError::Rejected(String::from("purge target is not an actor"))
                    })?;
                let mut target_data: ActorData = serde_json::from_value(target_raw)
                    .map_err(|error| SystemError::Failed(format!("corrupt target: {error}")))?;
                if !target_data.alive {
                    return Err(SystemError::Rejected(String::from(
                        "target is already dead",
                    )));
                }
                target_data.alive = false;
                target_data.power = 0;
                transaction.push_change(StateChange::SetProperty {
                    item: target,
                    key: actor_key(),
                    data: serde_json::to_value(&target_data)
                        .map_err(|error| SystemError::Failed(error.to_string()))?,
                });
                state_data.stability = (state_data.stability - 2).max(0);
                actor_data.power += 1;
            }
            ActionWire::Share => {
                state_data.stability = (state_data.stability + 1).min(10);
                actor_data.power = (actor_data.power - 1).max(0);
            }
            ActionWire::Hoard => {
                state_data.stability = (state_data.stability - 1).max(0);
                actor_data.power += 2;
            }
        }

        transaction.push_change(StateChange::SetProperty {
            item: state,
            key: state_key(),
            data: serde_json::to_value(&state_data)
                .map_err(|error| SystemError::Failed(error.to_string()))?,
        });
        transaction.push_change(StateChange::SetProperty {
            item: actor,
            key: actor_key(),
            data: serde_json::to_value(&actor_data)
                .map_err(|error| SystemError::Failed(error.to_string()))?,
        });
        Ok(transaction)
    }
}
