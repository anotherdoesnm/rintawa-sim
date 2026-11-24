//! Deterministic end-to-end run of the scripted baseline.

use rintawa_sim::{Policy, Scenario, ScenarioActor, SimDriver};

fn scenario() -> Scenario {
    Scenario {
        stability: 6,
        actors: vec![
            ScenarioActor {
                name: String::from("kitten"),
                power: 4,
                policy: Policy::Aggressive,
            },
            ScenarioActor {
                name: String::from("doe"),
                power: 3,
                policy: Policy::Opportunist,
            },
            ScenarioActor {
                name: String::from("mart"),
                power: 2,
                policy: Policy::Cautious,
            },
        ],
    }
}

#[test]
fn test_scripted_run_keeps_world_invariants() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let mut driver = SimDriver::start(&directory.path().join("world.sqlite"), &scenario())?;
    driver.run(6)?;
    let report = driver.finish()?;

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
    let first = {
        let directory = tempfile::tempdir()?;
        let mut driver = SimDriver::start(&directory.path().join("world.sqlite"), &scenario())?;
        driver.run(4)?;
        driver.finish()?
    };
    let second = {
        let directory = tempfile::tempdir()?;
        let mut driver = SimDriver::start(&directory.path().join("world.sqlite"), &scenario())?;
        driver.run(4)?;
        driver.finish()?
    };

    assert_eq!(first, second);
    Ok(())
}
