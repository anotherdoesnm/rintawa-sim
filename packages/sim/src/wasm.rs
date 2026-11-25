//! Component-model adapter for the simulation package.
//!
//! STATUS: skeleton. Three things must be pinned before this file can compile, and
//! all three belong to the rtwKit side of the contract:
//!
//! 1. `wit/engine.wit` is not vendored here. The guest must be generated against the
//!    exact engine world from the rtwKit branch that the package targets
//!    (`task-runtime-plugin`, branch `feat/mvp-ai-narrator`), not against a copy.
//! 2. The exported world-system entry point and the service-protocol envelope names
//!    must match that WIT; guessing them would produce a package that loads and then
//!    answers nothing.
//! 3. Inference must not be called from here. Per the narrator/AI contract only the
//!    World's selected Narrator may start inference, so the coordinator asks the
//!    Narrator (or the AI Provider, once per-request connection selection lands) and
//!    this adapter only turns the answer into `sim.act@1` commands.
//!
//! What is already fixed and can be reused without changes: the property and command
//! schemas, the two Systems (`sim.seed@1`, `sim.act@1`), the rule set, the policies
//! and the report model — they live in `rintawa-sim`.

#![allow(dead_code)]

/// Command payload the coordinator submits for one actor turn.
///
/// This mirrors `rintawa_sim::system::ActPayload` and exists here only to document the
/// boundary; the adapter reuses the crate type directly once the WIT is in place.
pub struct ActBoundary {
    /// Identity of the world-state item.
    pub state: String,
    /// Identity of the acting item.
    pub actor: String,
    /// Action name as it appears on the wire.
    pub action: String,
    /// Purge target, when the action needs one.
    pub target: Option<String>,
}

/// Builds the recommendation the coordinator hands to inference for one actor.
///
/// The prompt is produced by the crate, not by this adapter, so a scripted run and a
/// model-driven run ask exactly the same question.
pub fn recommendation_prompt(view: &rintawa_sim::WorldView, me: &rintawa_sim::ActorView) -> String {
    let others: Vec<String> = view
        .actors
        .iter()
        .filter(|actor| actor.id != me.id)
        .map(|actor| format!("{} power={} ruler={}", actor.name, actor.power, actor.is_ruler))
        .collect();
    format!(
        "round {} rules {:?} stability {} you {} power {} others [{}]",
        view.round,
        view.rules,
        view.stability,
        me.name,
        me.power,
        others.join("; ")
    )
}
