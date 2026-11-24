//! Scripted baseline for agent simulations on authoritative Rintawa worlds.
//!
//! The crate is a laboratory, not a Core feature: it declares ordinary property
//! definitions and command schemas, evaluates them with ordinary World Systems and
//! drives a real single-writer world runtime with deterministic policies. Nothing
//! here adds tables, jobs, a clock or replay to the world model.

#![forbid(unsafe_code)]
#![warn(missing_docs, rustdoc::broken_intra_doc_links)]

pub mod driver;
pub mod llm;
pub mod model;
pub mod schemas;
pub mod system;
pub mod template;

pub use driver::{Scenario, ScenarioActor, SimDriver, package_owner};
pub use llm::LlmPolicy;
pub use model::{
    ActionKind, ActorView, Aggressive, Cautious, Loyalist, Opportunist, PlannedAction, Policy,
    PolicyDecision, RoundReport, SimulationReport, WorldView,
};
pub use template::{MAX_TEMPLATE_ACTORS, ScenarioTemplate, TEMPLATE_SCHEMA, TemplateActor};
