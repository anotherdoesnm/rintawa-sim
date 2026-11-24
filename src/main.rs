//! Command line entry point for one scripted simulation run.

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use rintawa_sim::{
    Aggressive, Cautious, LlmConfig, LlmPolicy, Opportunist, Scenario, ScenarioActor,
    ScenarioTemplate, SimDriver,
};

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
    let mut use_llm = false;
    let mut template_in: Option<PathBuf> = None;
    let mut template_out: Option<PathBuf> = None;
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
            "--llm" => use_llm = true,
            "--template" => {
                template_in =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--template needs a value")
                    })?));
            }
            "--save-template" => {
                template_out =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--save-template needs a value")
                    })?));
            }
            "--help" | "-h" => {
                println!(
                    "usage: rintawa-sim [--rounds N] [--db PATH] [--llm] [--template PATH] [--save-template PATH]"
                );
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown argument {other}")),
        }
    }

    let mut scenario = default_scenario(use_llm)?;
    if let Some(path) = &template_in {
        let text = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", path.display()))?;
        scenario = ScenarioTemplate::from_json(&text)?.to_scenario()?;
    }
    if let Some(path) = &template_out {
        let template = ScenarioTemplate::from_scenario("court", &scenario);
        std::fs::write(path, template.to_json()?)
            .map_err(|error| anyhow::anyhow!("cannot write {}: {error}", path.display()))?;
        println!("wrote template to {}", path.display());
        return Ok(());
    }

    let mut driver = SimDriver::start(&database, &scenario)?;
    driver.run(rounds)?;
    let report = driver.finish()?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn default_scenario(use_llm: bool) -> anyhow::Result<Scenario> {
    let mut llm = None;
    let first_policy: Arc<dyn rintawa_sim::Policy> = if use_llm {
        let config = LlmConfig::new(
            std::env::var(rintawa_sim::llm::ENDPOINT_ENV)
                .map_err(|_| anyhow::anyhow!("{} is not set", rintawa_sim::llm::ENDPOINT_ENV))?,
            std::env::var(rintawa_sim::llm::MODEL_ENV)
                .map_err(|_| anyhow::anyhow!("{} is not set", rintawa_sim::llm::MODEL_ENV))?,
        )
        .with_key_env(rintawa_sim::llm::KEY_ENV);
        let policy = LlmPolicy::from_config(&config);
        llm = Some(config);
        Arc::new(policy)
    } else {
        Arc::new(Aggressive)
    };
    Ok(Scenario {
        stability: 6,
        actors: vec![
            ScenarioActor {
                name: String::from("kitten"),
                power: 4,
                loyal: false,
                policy: first_policy,
                llm,
            },
            ScenarioActor {
                name: String::from("doe"),
                power: 3,
                loyal: false,
                policy: Arc::new(Opportunist),
                llm: None,
            },
            ScenarioActor {
                name: String::from("mart"),
                power: 2,
                loyal: false,
                policy: Arc::new(Cautious),
                llm: None,
            },
        ],
    })
}
