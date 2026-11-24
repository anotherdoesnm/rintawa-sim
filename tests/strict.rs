//! Rule enforcement lives in the world, so these tests read the refusals back.

use std::sync::Arc;

use rintawa_sim::{
    ActionKind, ActorView, PlannedAction, Policy, PolicyDecision, Rules, Scenario, ScenarioActor,
    SimDriver, WorldView,
};

/// Takes the empty throne once, then only shares: it never purges, so the other
/// actor always gets its attempt at a forbidden action.
struct Usurper;

impl Policy for Usurper {
    fn name(&self) -> &str {
        "usurper"
    }

    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision {
        if view.ruler.is_none() {
            PolicyDecision::Act(PlannedAction::new(ActionKind::GrabPower))
        } else {
            PolicyDecision::Act(PlannedAction::new(ActionKind::Share))
        }
    }
}

/// Always attempts the same action, optionally targeting the ruler.
struct Always {
    action: ActionKind,
    target_ruler: bool,
}

impl Policy for Always {
    fn name(&self) -> &str {
        "always"
    }

    fn decide(&self, view: &WorldView, me: &ActorView) -> PolicyDecision {
        let target = if self.target_ruler {
            view.ruler.filter(|ruler| *ruler != me.id)
        } else {
            None
        };
        PolicyDecision::Act(match self.action {
            ActionKind::Purge => match target.or_else(|| view.weakest_rival(me)) {
                Some(target) => PlannedAction::purge(target),
                None => PlannedAction::new(ActionKind::Share),
            },
            other => PlannedAction::new(other),
        })
    }
}

fn actor(name: &str, power: i64, policy: Arc<dyn Policy>) -> ScenarioActor {
    ScenarioActor {
        name: String::from(name),
        power,
        loyal: false,
        policy,
        llm: None,
    }
}

fn run_scenario(scenario: Scenario, rounds: u32) -> anyhow::Result<rintawa_sim::SimulationReport> {
    let directory = tempfile::tempdir()?;
    let mut driver = SimDriver::start(&directory.path().join("world.sqlite"), &scenario)?;
    driver.run(rounds)?;
    driver.finish()
}

/// Reasons of every command the world refused.
fn refusals(report: &rintawa_sim::SimulationReport) -> Vec<String> {
    report
        .actors
        .iter()
        .flat_map(|actor| actor.decisions.iter())
        .filter(|decision| decision.action == "refused")
        .filter_map(|decision| decision.note.clone())
        .collect()
}

#[test]
fn test_should_refuse_a_coup_without_more_power_than_the_ruler() -> anyhow::Result<()> {
    let scenario = Scenario {
        rules: Rules::Strict,
        stability: 6,
        actors: vec![
            actor("ruler", 5, Arc::new(Usurper)),
            actor(
                "weak",
                1,
                Arc::new(Always {
                    action: ActionKind::GrabPower,
                    target_ruler: false,
                }),
            ),
        ],
    };

    let report = run_scenario(scenario, 3)?;
    let refusals = refusals(&report);

    assert!(
        refusals
            .iter()
            .any(|note| note.contains("strictly more power")),
        "the weak actor must be refused: {refusals:?}"
    );
    assert_eq!(report.ruler.as_deref(), Some("ruler"));
    Ok(())
}

#[test]
fn test_should_refuse_a_purge_without_enough_power() -> anyhow::Result<()> {
    let scenario = Scenario {
        rules: Rules::Strict,
        stability: 6,
        actors: vec![
            actor("strong", 5, Arc::new(Usurper)),
            actor(
                "weak",
                1,
                Arc::new(Always {
                    action: ActionKind::Purge,
                    target_ruler: false,
                }),
            ),
        ],
    };

    let report = run_scenario(scenario, 3)?;
    let refusals = refusals(&report);

    assert!(
        refusals
            .iter()
            .any(|note| note.contains("at least as much power")),
        "the weak actor must be refused: {refusals:?}"
    );
    assert_eq!(report.deaths, 0, "nobody dies under the strict power rule");
    Ok(())
}

#[test]
fn test_should_refuse_political_actions_at_zero_stability() -> anyhow::Result<()> {
    let scenario = Scenario {
        rules: Rules::Strict,
        stability: 0,
        actors: vec![actor("actor", 3, Arc::new(Usurper))],
    };

    let report = run_scenario(scenario, 1)?;
    let refusals = refusals(&report);

    assert!(
        refusals
            .iter()
            .any(|note| note.contains("stability is zero")),
        "no action may be accepted at zero stability: {refusals:?}"
    );
    assert_eq!(report.ruler, None);
    Ok(())
}

#[test]
fn test_should_leave_the_throne_vacant_when_the_ruler_is_purged() -> anyhow::Result<()> {
    let scenario = Scenario {
        rules: Rules::Strict,
        stability: 8,
        actors: vec![
            actor("ruler", 3, Arc::new(Usurper)),
            actor(
                "killer",
                6,
                Arc::new(Always {
                    action: ActionKind::Purge,
                    target_ruler: true,
                }),
            ),
        ],
    };

    let report = run_scenario(scenario, 1)?;

    assert_eq!(report.ruler, None, "a purged ruler leaves no heir");
    assert_eq!(report.deaths, 1);
    Ok(())
}
