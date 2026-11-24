//! World Systems that evaluate the simulation commands into item/property changes.

use rintawa_sdk::world::{ItemId, SchemaKey};
use rintawa_world::{StateChange, WorldTransaction};
use rintawa_world_runtime::{SystemError, SystemResult, WorldSnapshot, WorldSystem};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::Rules;
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
    /// Rule set the world will enforce.
    #[serde(default)]
    pub rules: Rules,
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
    /// Whether the actor executes orders instead of competing for power.
    pub loyal: bool,
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
    #[serde(default)]
    rules: Rules,
}

/// Actor property payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActorData {
    name: String,
    power: i64,
    alive: bool,
    loyal: bool,
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
            data: json!({
                "stability": payload.stability,
                "ruler": Value::Null,
                "rules": payload.rules
            }),
        });

        for actor in &payload.actors {
            let id = parse_item(&actor.id, "actor")?;
            transaction.push_change(StateChange::CreateObject { id });
            transaction.push_change(StateChange::SetProperty {
                item: id,
                key: actor_key(),
                data: json!({
                    "name": actor.name,
                    "power": actor.power,
                    "alive": true,
                    "loyal": actor.loyal
                }),
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

        // At zero stability the realm tolerates no politics, but sharing is what
        // restores it, so that one action stays available.
        if matches!(state_data.rules, Rules::Strict)
            && state_data.stability == 0
            && !matches!(payload.action, ActionWire::Share)
        {
            return Err(SystemError::Rejected(String::from(
                "stability is zero: only sharing power is accepted, no coup, purge or hoard",
            )));
        }

        let mut transaction = WorldTransaction::new();
        match payload.action {
            ActionWire::GrabPower => {
                if matches!(state_data.rules, Rules::Strict)
                    && let Some(ruler) = state_data.ruler.as_deref()
                    && let Ok(ruler_id) = ruler.parse::<ItemId>()
                    && let Some(ruler_raw) = read_property(snapshot, ruler_id, &actor_key())?
                    && let Ok(ruler_data) = serde_json::from_value::<ActorData>(ruler_raw)
                    && ruler_data.alive
                    && actor_data.power <= ruler_data.power
                {
                    return Err(SystemError::Rejected(format!(
                        "a coup needs strictly more power than the ruler: {} vs {}",
                        actor_data.power, ruler_data.power
                    )));
                }
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
                if matches!(state_data.rules, Rules::Strict) {
                    if actor_data.power < target_data.power {
                        return Err(SystemError::Rejected(format!(
                            "a purge needs at least as much power as its target: {} vs {}",
                            actor_data.power, target_data.power
                        )));
                    }
                    actor_data.power = (actor_data.power - 2).max(0);
                    if state_data.ruler.as_deref() == Some(target.to_string().as_str()) {
                        state_data.ruler = None;
                    }
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
