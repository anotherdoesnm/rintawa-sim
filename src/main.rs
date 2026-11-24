//! Command line entry point for one scripted simulation run.

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use rintawa_sim::{Aggressive, Cautious, Opportunist, Scenario, ScenarioActor, SimDriver};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("simulation failed: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let mut rounds = 8_u32;
    let mut database =
        std::env::temp_dir().join(format!("rintawa-sim-{}.sqlite", std::process::id()));
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rounds" => {
                rounds = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--rounds needs a value"))?
                    .parse()?;
            }
            "--db" => {
                database = PathBuf::from(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--db needs a value"))?,
                );
            }
            "--help" | "-h" => {
                println!("usage: rintawa-sim [--rounds N] [--db PATH]");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown argument {other}")),
        }
    }

    let scenario = Scenario {
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
    };

    let mut driver = SimDriver::start(&database, &scenario)?;
    driver.run(rounds)?;
    let report = driver.finish()?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
