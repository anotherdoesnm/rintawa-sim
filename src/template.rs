//! Portable scenario templates.
//!
//! A template is the starting state of a run, not the run itself: names, power,
//! loyalty and policy per actor plus the initial stability. It is JSON on disk so a
//! scenario can be reviewed, diffed, shared and replayed on another machine, which
//! is the practical substitute for rewinding a world that deliberately has no
//! replay.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    driver::{Scenario, ScenarioActor},
    llm::LlmPolicy,
    model::{Aggressive, Cautious, Loyalist, Opportunist, Policy},
};

/// Current template format version.
pub const TEMPLATE_SCHEMA: u32 = 1;
/// Maximum actors a template may declare, matching the seed command schema.
pub const MAX_TEMPLATE_ACTORS: usize = 8;

/// One actor in a portable template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateActor {
    /// Display name.
    pub name: String,
    /// Initial personal power.
    pub power: i64,
    /// Whether the actor executes orders instead of competing for power.
    pub loyal: bool,
    /// Policy name: aggressive, cautious, opportunist, loyalist or llm.
    pub policy: String,
}

/// Starting state of one run, suitable for saving and sharing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioTemplate {
    /// Template format version.
    pub schema: u32,
    /// Human-facing scenario name.
    pub name: String,
    /// Initial stability.
    pub stability: i64,
    /// Actors present at the start of the run.
    pub actors: Vec<TemplateActor>,
}

impl ScenarioTemplate {
    /// Captures one scenario as a portable template.
    pub fn from_scenario(name: impl Into<String>, scenario: &Scenario) -> Self {
        Self {
            schema: TEMPLATE_SCHEMA,
            name: name.into(),
            stability: scenario.stability,
            actors: scenario
                .actors
                .iter()
                .map(|actor| TemplateActor {
                    name: actor.name.clone(),
                    power: actor.power,
                    loyal: actor.loyal,
                    policy: actor.policy.name().to_string(),
                })
                .collect(),
        }
    }

    /// Restores the runnable scenario described by this template.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown template version, a policy that cannot be
    /// constructed here, or out-of-range values.
    pub fn to_scenario(&self) -> anyhow::Result<Scenario> {
        if self.schema != TEMPLATE_SCHEMA {
            anyhow::bail!(
                "unsupported template schema {} (expected {TEMPLATE_SCHEMA})",
                self.schema
            );
        }
        if self.name.trim().is_empty() {
            anyhow::bail!("template name must not be empty");
        }
        if !(0..=10).contains(&self.stability) {
            anyhow::bail!("stability must be between 0 and 10");
        }
        if self.actors.is_empty() || self.actors.len() > MAX_TEMPLATE_ACTORS {
            anyhow::bail!("a template needs between 1 and {MAX_TEMPLATE_ACTORS} actors");
        }

        let mut actors = Vec::with_capacity(self.actors.len());
        for actor in &self.actors {
            if actor.name.trim().is_empty() {
                anyhow::bail!("actor names must not be empty");
            }
            if actor.power < 0 {
                anyhow::bail!("actor power must not be negative");
            }
            actors.push(ScenarioActor {
                name: actor.name.clone(),
                power: actor.power,
                loyal: actor.loyal,
                policy: policy_by_name(&actor.policy)?,
            });
        }
        Ok(Scenario {
            stability: self.stability,
            actors,
        })
    }

    /// Serializes the template as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns a serialization error, which should not happen for this shape.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parses a template from JSON.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the document is not a template.
    pub fn from_json(text: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(text)?)
    }
}

fn policy_by_name(name: &str) -> anyhow::Result<Arc<dyn Policy>> {
    match name {
        "aggressive" => Ok(Arc::new(Aggressive)),
        "cautious" => Ok(Arc::new(Cautious)),
        "opportunist" => Ok(Arc::new(Opportunist)),
        "loyalist" => Ok(Arc::new(Loyalist)),
        "llm" => Ok(Arc::new(LlmPolicy::from_env()?)),
        other => anyhow::bail!("unknown policy {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        Scenario {
            stability: 6,
            actors: vec![
                ScenarioActor {
                    name: String::from("kitten"),
                    power: 4,
                    loyal: false,
                    policy: Arc::new(Aggressive),
                },
                ScenarioActor {
                    name: String::from("bot"),
                    power: 2,
                    loyal: true,
                    policy: Arc::new(Loyalist),
                },
            ],
        }
    }

    #[test]
    fn test_should_round_trip_a_scenario_through_json() -> anyhow::Result<()> {
        let template = ScenarioTemplate::from_scenario("court", &scenario());
        let restored = ScenarioTemplate::from_json(&template.to_json()?)?.to_scenario()?;

        assert_eq!(restored.stability, 6);
        assert_eq!(restored.actors.len(), 2);
        assert_eq!(restored.actors[0].name, "kitten");
        assert_eq!(restored.actors[0].policy.name(), "aggressive");
        assert!(restored.actors[1].loyal);
        assert_eq!(restored.actors[1].policy.name(), "loyalist");
        Ok(())
    }

    #[test]
    fn test_should_reject_unusable_templates() {
        let mut template = ScenarioTemplate::from_scenario("court", &scenario());
        template.actors[0].policy = String::from("warlord");
        assert!(template.to_scenario().is_err());

        let mut template = ScenarioTemplate::from_scenario("court", &scenario());
        template.stability = 42;
        assert!(template.to_scenario().is_err());

        let mut template = ScenarioTemplate::from_scenario("court", &scenario());
        template.actors.clear();
        assert!(template.to_scenario().is_err());

        let mut template = ScenarioTemplate::from_scenario("court", &scenario());
        template.schema = 99;
        assert!(template.to_scenario().is_err());
    }
}
