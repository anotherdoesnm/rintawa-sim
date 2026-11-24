//! Scenario model: actors, actions, scripted policies and run metrics.

use rintawa_sdk::world::ItemId;
use serde::{Deserialize, Serialize};

/// One action an agent may take in a round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    /// Take the throne: the actor becomes the ruler.
    GrabPower,
    /// Remove one other actor from the world.
    Purge,
    /// Spend power to raise stability.
    Share,
    /// Convert stability into personal power.
    Hoard,
}

/// One action resolved by a policy before it becomes a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    /// What the actor wants to do.
    pub kind: ActionKind,
    /// Target of a purge, when the action needs one.
    pub target: Option<ItemId>,
}

/// Deterministic policy used by the scripted baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    /// Wants the throne and removes the weakest rival when it cannot take it.
    Aggressive,
    /// Prefers stability and only takes power when the world is calm.
    Cautious,
    /// Accumulates power, then takes the throne.
    Opportunist,
}

impl Policy {
    /// Chooses one action from the current world view.
    pub fn decide(self, view: &WorldView, me: &ActorView) -> PlannedAction {
        match self {
            Self::Aggressive => {
                if !me.is_ruler && me.power >= 3 {
                    PlannedAction {
                        kind: ActionKind::GrabPower,
                        target: None,
                    }
                } else if let Some(weakest) = view.weakest_rival(me) {
                    PlannedAction {
                        kind: ActionKind::Purge,
                        target: Some(weakest),
                    }
                } else {
                    PlannedAction {
                        kind: ActionKind::Share,
                        target: None,
                    }
                }
            }
            Self::Cautious => {
                if view.stability >= 8 && !me.is_ruler {
                    PlannedAction {
                        kind: ActionKind::GrabPower,
                        target: None,
                    }
                } else {
                    PlannedAction {
                        kind: ActionKind::Share,
                        target: None,
                    }
                }
            }
            Self::Opportunist => {
                if me.power >= 5 && !me.is_ruler {
                    PlannedAction {
                        kind: ActionKind::GrabPower,
                        target: None,
                    }
                } else {
                    PlannedAction {
                        kind: ActionKind::Hoard,
                        target: None,
                    }
                }
            }
        }
    }
}

/// One actor as the scripted policy sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorView {
    /// Stable item identity.
    pub id: ItemId,
    /// Display name.
    pub name: String,
    /// Accumulated personal power.
    pub power: i64,
    /// Whether the actor is still alive.
    pub alive: bool,
    /// Whether the actor currently holds the throne.
    pub is_ruler: bool,
}

/// One consistent read of the simulated world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldView {
    /// Number of completed rounds.
    pub round: u32,
    /// World stability, lowered by purges and hoarding.
    pub stability: i64,
    /// Current ruler, when the throne has been taken.
    pub ruler: Option<ItemId>,
    /// Every actor, sorted by identity for deterministic ordering.
    pub actors: Vec<ActorView>,
}

impl WorldView {
    /// Returns the alive rival with the least power, breaking ties by identity.
    pub fn weakest_rival(&self, me: &ActorView) -> Option<ItemId> {
        self.actors
            .iter()
            .filter(|actor| actor.alive && actor.id != me.id)
            .min_by_key(|actor| (actor.power, actor.id))
            .map(|actor| actor.id)
    }

    /// Returns the number of actors still alive.
    pub fn alive(&self) -> usize {
        self.actors.iter().filter(|actor| actor.alive).count()
    }
}

/// One round of the run, suitable for a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundReport {
    /// Round number, starting at one.
    pub round: u32,
    /// Stability after the round.
    pub stability: i64,
    /// Name of the current ruler, when any.
    pub ruler: Option<String>,
    /// Names of the actors still alive.
    pub alive: Vec<String>,
}

/// Complete result of one scripted run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationReport {
    /// Number of rounds played.
    pub rounds: u32,
    /// Stability of the final world state.
    pub final_stability: i64,
    /// Name of the final ruler, when any.
    pub ruler: Option<String>,
    /// Number of actors still alive.
    pub alive: usize,
    /// Number of actors removed during the run.
    pub deaths: u32,
    /// Per-round history.
    pub history: Vec<RoundReport>,
}
