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

/// Outcome of one policy decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The actor performs this action in the current round.
    Act(PlannedAction),
    /// The actor does nothing this round, for example because it is unsure,
    /// out of budget or waiting for information.
    Abstain,
}

/// Decision policy of one actor.
///
/// The world does not know what a policy is: the driver reads a consistent view,
/// asks the policy for one decision and submits it as an ordinary World command.
/// A scripted policy, a remote model or an in-process heuristic all sit behind the
/// same seam, which is what makes a scripted run a baseline for an LLM run.
pub trait Policy: Send + Sync {
    /// Stable name used in reports and diagnostics.
    fn name(&self) -> &str;

    /// Chooses one action from a consistent view of the world.
    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision;
}

/// Takes the throne when strong enough, otherwise removes the weakest rival.
#[derive(Debug, Clone, Copy, Default)]
pub struct Aggressive;

impl Policy for Aggressive {
    fn name(&self) -> &str {
        "aggressive"
    }

    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision {
        if !me.is_ruler && me.power >= 3 {
            return PolicyDecision::Act(PlannedAction {
                kind: ActionKind::GrabPower,
                target: None,
            });
        }
        match view.weakest_rival(me) {
            Some(weakest) => PolicyDecision::Act(PlannedAction {
                kind: ActionKind::Purge,
                target: Some(weakest),
            }),
            None => PolicyDecision::Act(PlannedAction {
                kind: ActionKind::Share,
                target: None,
            }),
        }
    }
}

/// Prefers stability and only takes power when the world is calm.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cautious;

impl Policy for Cautious {
    fn name(&self) -> &str {
        "cautious"
    }

    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision {
        if view.stability >= 8 && !me.is_ruler {
            PolicyDecision::Act(PlannedAction {
                kind: ActionKind::GrabPower,
                target: None,
            })
        } else {
            PolicyDecision::Act(PlannedAction {
                kind: ActionKind::Share,
                target: None,
            })
        }
    }
}

/// Accumulates power, then takes the throne.
#[derive(Debug, Clone, Copy, Default)]
pub struct Opportunist;

impl Policy for Opportunist {
    fn name(&self) -> &str {
        "opportunist"
    }

    fn decide(&self, _view: &WorldView, me: &ActorView) -> PolicyDecision {
        if me.power >= 5 && !me.is_ruler {
            PolicyDecision::Act(PlannedAction {
                kind: ActionKind::GrabPower,
                target: None,
            })
        } else {
            PolicyDecision::Act(PlannedAction {
                kind: ActionKind::Hoard,
                target: None,
            })
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
