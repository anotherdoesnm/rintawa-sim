//! RTW guest adapter for the simulation package.
//!
//! The simulation logic (schemas, Systems, policies, reports) lives in the `rintawa-sim`
//! crate this adapter depends on. This crate only translates between the component
//! model and that logic; see `src/wasm.rs`.
