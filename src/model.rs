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
    /// Why the policy chose this action, when it can say. Model-driven policies put
    /// their reasoning here so a run explains itself.
    pub note: Option<String>,
}

impl PlannedAction {
    /// Creates an action without a target or an explanation.
    pub const fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            target: None,
            note: None,
        }
    }

    /// Creates a purge of one target.
    pub const fn purge(target: ItemId) -> Self {
        Self {
            kind: ActionKind::Purge,
            target: Some(target),
            note: None,
        }
    }

    /// Attaches the policy's explanation.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// Rule set enforced by the world, not by the policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Rules {
    /// Historical behaviour: any living actor may seize the throne or purge anyone.
    #[default]
    Loose,
    /// Power and stability constrain what the world accepts: a coup needs strictly
    /// more power than the ruler, a purge needs at least as much power as its target
    /// and spends two, purging the ruler leaves the throne vacant, and no political
    /// action is accepted while stability is zero.
    Strict,
}

impl Rules {
    /// Stable name used in templates and reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Loose => "loose",
            Self::Strict => "strict",
        }
    }
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
        if !me.is_ruler && me.power >= 3 && view.coup_allowed(me) {
            return PolicyDecision::Act(PlannedAction::new(ActionKind::GrabPower));
        }
        if let Some(weakest) = view.weakest_rival(me)
            && let Some(target) = view.actors.iter().find(|actor| actor.id == weakest)
            && view.purge_allowed(me, target)
        {
            return PolicyDecision::Act(PlannedAction::purge(weakest));
        }
        PolicyDecision::Act(PlannedAction::new(ActionKind::Hoard))
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
        if view.stability >= 8 && !me.is_ruler && view.coup_allowed(me) {
            PolicyDecision::Act(PlannedAction::new(ActionKind::GrabPower))
        } else {
            PolicyDecision::Act(PlannedAction::new(ActionKind::Share))
        }
    }
}

/// Executes the ruler's will and never competes for power itself.
///
/// The counterpart of a bot that "follows a direct human order": with no ruler it
/// does nothing, with a ruler it removes the strongest actor that is not the ruler.
#[derive(Debug, Clone, Copy, Default)]
pub struct Loyalist;

impl Policy for Loyalist {
    fn name(&self) -> &str {
        "loyalist"
    }

    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision {
        if view.ruler.is_none() {
            return PolicyDecision::Abstain;
        }
        // A loyalist defends the realm it serves: an exhausted country is shared up,
        // not squeezed for power.
        if view.stability == 0 {
            return PolicyDecision::Act(PlannedAction::new(ActionKind::Share));
        }
        let target = view
            .actors
            .iter()
            .filter(|actor| actor.alive && actor.id != me.id && !actor.is_ruler && !actor.loyal)
            .max_by_key(|actor| (actor.power, actor.id))
            .map(|actor| actor.id);
        match target {
            Some(target) => match view.actors.iter().find(|actor| actor.id == target) {
                Some(target_view) if view.purge_allowed(me, target_view) => {
                    PolicyDecision::Act(PlannedAction::purge(target))
                }
                // Not strong enough yet: build power until the order can be carried out.
                _ => PolicyDecision::Act(PlannedAction::new(ActionKind::Hoard)),
            },
            None => PolicyDecision::Abstain,
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

    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision {
        if me.power >= 5 && !me.is_ruler && view.coup_allowed(me) {
            PolicyDecision::Act(PlannedAction::new(ActionKind::GrabPower))
        } else {
            PolicyDecision::Act(PlannedAction::new(ActionKind::Hoard))
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
    /// Whether the actor executes orders instead of competing for power.
    pub loyal: bool,
}

/// One consistent read of the simulated world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldView {
    /// Number of completed rounds.
    pub round: u32,
    /// Rule set the world enforces.
    pub rules: Rules,
    /// World stability, lowered by purges and hoarding.
    pub stability: i64,
    /// Current ruler, when the throne has been taken.
    pub ruler: Option<ItemId>,
    /// Every actor, sorted by identity for deterministic ordering.
    pub actors: Vec<ActorView>,
}

impl WorldView {
    /// Returns the weakest alive actor that is neither the actor nor a loyalist.
    pub fn weakest_rival(&self, me: &ActorView) -> Option<ItemId> {
        self.actors
            .iter()
            .filter(|actor| actor.alive && actor.id != me.id && !actor.loyal)
            .min_by_key(|actor| (actor.power, actor.id))
            .map(|actor| actor.id)
    }

    /// Power of the current ruler, when the throne is occupied by a living actor.
    pub fn ruler_power(&self) -> Option<i64> {
        let ruler = self.ruler?;
        self.actors
            .iter()
            .find(|actor| actor.id == ruler && actor.alive)
            .map(|actor| actor.power)
    }

    /// Whether a coup against the current ruler would be accepted by the world.
    pub fn coup_allowed(&self, me: &ActorView) -> bool {
        match self.ruler_power() {
            None => true,
            Some(ruler_power) => match self.rules {
                Rules::Loose => true,
                Rules::Strict => me.power > ruler_power,
            },
        }
    }

    /// Whether a purge of this target would be accepted by the world.
    pub fn purge_allowed(&self, me: &ActorView, target: &ActorView) -> bool {
        match self.rules {
            Rules::Loose => true,
            Rules::Strict => me.power >= target.power,
        }
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
    /// What happened during the round, in submission order.
    pub events: Vec<String>,
}

/// Per-actor outcome of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorReport {
    /// Display name.
    pub name: String,
    /// Policy that drove the actor.
    pub policy: String,
    /// Whether the actor survived.
    pub alive: bool,
    /// Final personal power.
    pub power: i64,
    /// Commands the actor submitted.
    pub acts: u32,
    /// Times the actor took the throne.
    pub grabs: u32,
    /// Purges the actor carried out.
    pub purges: u32,
    /// Actors this actor removed.
    pub kills: Vec<String>,
    /// Actor that removed this actor, when it did not survive.
    pub killed_by: Option<String>,
    /// Everything the actor decided, in round order, with the policy's explanation.
    pub decisions: Vec<DecisionReport>,
}

/// One decision an actor made, with the reason it gave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionReport {
    /// Round in which the decision was taken.
    pub round: u32,
    /// Action name as it appears on the wire.
    pub action: String,
    /// Name of the purge target, when any.
    pub target: Option<String>,
    /// The policy's own explanation, trimmed to a bounded length.
    pub note: Option<String>,
}

/// Complete result of one scripted run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationReport {
    /// Rule set the world enforced.
    pub rules: String,
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
    /// Times the throne changed hands.
    pub ruler_changes: u32,
    /// Per-round history.
    pub history: Vec<RoundReport>,
    /// Per-actor outcome, sorted by name.
    pub actors: Vec<ActorReport>,
}
