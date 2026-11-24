//! Deterministic end-to-end runs of the scripted baseline and of pluggable policies.

use std::sync::Arc;

use rintawa_sim::{
    ActionKind, ActorView, Aggressive, Cautious, Opportunist, PlannedAction, Policy,
    PolicyDecision, Scenario, ScenarioActor, SimDriver, WorldView,
};

/// Agent that always spends power on stability.
struct Pacifist;

impl Policy for Pacifist {
    fn name(&self) -> &str {
        "pacifist"
    }

    fn decide(&self, _view: &WorldView, _me: &ActorView) -> PolicyDecision {
        PolicyDecision::Act(PlannedAction {
            kind: ActionKind::Share,
            target: None,
        })
    }
}

/// Agent that never acts, standing in for a policy that is out of budget or unsure.
struct Idle;

impl Policy for Idle {
    fn name(&self) -> &str {
        "idle"
    }

    fn decide(&self, _view: &WorldView, _me: &ActorView) -> PolicyDecision {
        PolicyDecision::Abstain
    }
}

fn scenario() -> Scenario {
    Scenario {
        stability: 6,
        actors: vec![
            ScenarioActor {
                name: String::from("kitten"),
                power: 4,
                policy: Arc::new(Aggressive),
            },
            ScenarioActor {
                name: String::from("doe"),
                power: 3,
                policy: Arc::new(Opportunist),
            },
            ScenarioActor {
                name: String::from("mart"),
                power: 2,
                policy: Arc::new(Cautious),
            },
        ],
    }
}

fn run(scenario: &Scenario, rounds: u32) -> anyhow::Result<rintawa_sim::SimulationReport> {
    let directory = tempfile::tempdir()?;
    let mut driver = SimDriver::start(&directory.path().join("world.sqlite"), scenario)?;
    driver.run(rounds)?;
    driver.finish()
}

#[test]
fn test_scripted_run_keeps_world_invariants() -> anyhow::Result<()> {
    let report = run(&scenario(), 6)?;

    assert_eq!(report.rounds, 6);
    assert_eq!(report.history.len(), 6);
    assert!(report.alive >= 1, "at least one actor must survive");
    assert!(report.ruler.is_some(), "the throne is taken during the run");
    assert!(
        report.deaths >= 1,
        "the aggressive actor should purge at least once"
    );
    for round in &report.history {
        assert!(
            (0..=10).contains(&round.stability),
            "stability stays inside its schema bound"
        );
        assert!(!round.alive.is_empty());
    }
    Ok(())
}

#[test]
fn test_run_is_deterministic_for_the_same_scenario() -> anyhow::Result<()> {
    assert_eq!(run(&scenario(), 4)?, run(&scenario(), 4)?);
    Ok(())
}

#[test]
fn test_custom_policy_needs_no_world_changes() -> anyhow::Result<()> {
    let pacifist = Scenario {
        stability: 6,
        actors: (0..3)
            .map(|index| ScenarioActor {
                name: format!("pacifist-{index}"),
                power: 2,
                policy: Arc::new(Pacifist),
            })
            .collect(),
    };

    let report = run(&pacifist, 3)?;

    assert_eq!(report.deaths, 0, "nobody purges in a pacifist world");
    assert_eq!(report.ruler, None, "nobody takes the throne");
    assert_eq!(
        report.final_stability, 10,
        "stability saturates at its bound"
    );
    Ok(())
}

#[test]
fn test_abstaining_policy_leaves_the_world_untouched() -> anyhow::Result<()> {
    let idle = Scenario {
        stability: 6,
        actors: vec![ScenarioActor {
            name: String::from("idle"),
            power: 1,
            policy: Arc::new(Idle),
        }],
    };

    let report = run(&idle, 2)?;

    assert_eq!(report.rounds, 2);
    assert_eq!(report.final_stability, 6);
    assert_eq!(report.ruler, None);
    assert_eq!(report.deaths, 0);
    Ok(())
}

#[test]
fn test_report_attributes_actions_to_actors() -> anyhow::Result<()> {
    let report = run(&scenario(), 6)?;

    assert_eq!(report.actors.len(), 3);
    assert!(report.ruler_changes >= 1, "the throne changes hands");
    assert!(
        report.actors.iter().any(|actor| actor.acts > 0),
        "actors act during the run"
    );

    let dead: Vec<_> = report.actors.iter().filter(|actor| !actor.alive).collect();
    assert_eq!(dead.len() as u32, report.deaths);
    for actor in dead {
        let killer = actor
            .killed_by
            .as_ref()
            .expect("a removed actor records who removed it");
        let killer = report
            .actors
            .iter()
            .find(|candidate| &candidate.name == killer)
            .expect("the killer is part of the report");
        assert!(
            killer.kills.contains(&actor.name),
            "the killer records its victim"
        );
    }

    for round in &report.history {
        assert!(
            !round.events.is_empty(),
            "every scripted round records what happened"
        );
    }
    Ok(())
}
