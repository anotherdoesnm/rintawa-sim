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
    llm::{LlmConfig, LlmPolicy},
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
    /// Endpoint and model for the `llm` policy. Never contains a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmConfig>,
}

/// Starting state of one run, suitable for saving and sharing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioTemplate {
    /// Template format version.
    pub schema: u32,
    /// Human-facing scenario name.
    pub name: String,
    /// Rule set the world enforces.
    #[serde(default)]
    pub rules: crate::model::Rules,
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
            rules: scenario.rules,
            stability: scenario.stability,
            actors: scenario
                .actors
                .iter()
                .map(|actor| TemplateActor {
                    name: actor.name.clone(),
                    power: actor.power,
                    loyal: actor.loyal,
                    policy: if actor.llm.is_some() {
                        String::from("llm")
                    } else {
                        actor.policy.name().to_string()
                    },
                    llm: actor.llm.clone(),
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
            let (policy, llm) = policy_for(&actor.policy, actor.llm.as_ref(), &actor.name)?;
            actors.push(ScenarioActor {
                name: actor.name.clone(),
                power: actor.power,
                loyal: actor.loyal,
                policy,
                llm,
            });
        }
        Ok(Scenario {
            rules: self.rules,
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

fn policy_for(
    name: &str,
    llm: Option<&LlmConfig>,
    actor: &str,
) -> anyhow::Result<(Arc<dyn Policy>, Option<LlmConfig>)> {
    let policy: Arc<dyn Policy> = match name {
        "aggressive" => Arc::new(Aggressive),
        "cautious" => Arc::new(Cautious),
        "opportunist" => Arc::new(Opportunist),
        "loyalist" => Arc::new(Loyalist),
        "llm" => {
            let config = llm.ok_or_else(|| {
                anyhow::anyhow!("actor {actor} uses the llm policy but has no llm block")
            })?;
            Arc::new(LlmPolicy::from_config(config))
        }
        other => anyhow::bail!("unknown policy {other}"),
    };
    Ok((policy, llm.cloned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        Scenario {
            rules: crate::model::Rules::Loose,
            stability: 6,
            actors: vec![
                ScenarioActor {
                    name: String::from("kitten"),
                    power: 4,
                    loyal: false,
                    policy: Arc::new(Aggressive),
                    llm: None,
                },
                ScenarioActor {
                    name: String::from("bot"),
                    power: 2,
                    loyal: true,
                    policy: Arc::new(Loyalist),
                    llm: None,
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
    fn test_should_keep_per_actor_models_and_never_a_key() -> anyhow::Result<()> {
        let scenario = Scenario {
            rules: crate::model::Rules::Loose,
            stability: 6,
            actors: vec![ScenarioActor {
                name: String::from("kitten"),
                power: 4,
                loyal: false,
                policy: Arc::new(LlmPolicy::from_config(
                    &LlmConfig::new("https://glm.t3ks.com/v1/chat/completions", "glm-5.4")
                        .with_key_env("ZAI_API_KEY")
                        .with_budget(4096),
                )),
                llm: Some(
                    LlmConfig::new("https://glm.t3ks.com/v1/chat/completions", "glm-5.4")
                        .with_key_env("ZAI_API_KEY")
                        .with_budget(4096),
                ),
            }],
        };

        let json = ScenarioTemplate::from_scenario("mixed", &scenario).to_json()?;
        assert!(!json.contains("sk-"), "templates never carry a key");
        let restored = ScenarioTemplate::from_json(&json)?.to_scenario()?;

        assert_eq!(restored.actors[0].policy.name(), "llm:glm-5.4");
        assert_eq!(
            restored.actors[0]
                .llm
                .as_ref()
                .map(|llm| llm.model.as_str()),
            Some("glm-5.4")
        );
        assert_eq!(
            restored.actors[0]
                .llm
                .as_ref()
                .and_then(|llm| llm.key_env.as_deref()),
            Some("ZAI_API_KEY")
        );
        Ok(())
    }

    #[test]
    fn test_should_reject_llm_actor_without_configuration() {
        let mut template = ScenarioTemplate::from_scenario("court", &scenario());
        template.actors[0].policy = String::from("llm");
        template.actors[0].llm = None;
        let error = template.to_scenario().unwrap_err().to_string();
        assert!(
            error.contains("llm block"),
            "error explains the missing block: {error}"
        );
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
