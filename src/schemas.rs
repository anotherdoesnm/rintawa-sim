//! Versioned property definitions and command schemas owned by the simulation package.
//!
//! Properties are persisted world vocabulary; command schemas are transient runtime
//! API descriptions supplied by the Host and never stored in `property_registry`.

use rintawa_sdk::{
    types::ExtensionId,
    world::{SchemaId, SchemaKey, SchemaKind, SchemaVersion},
};
use rintawa_world::{PropertyDefinition, RuntimeSchemaDefinition};
use serde_json::json;

/// Unversioned property name of the shared world state item.
pub const STATE_PROPERTY: &str = "sim.state";
/// Unversioned property name carried by every actor item.
pub const ACTOR_PROPERTY: &str = "sim.actor";
/// Command that seeds actors and the world state.
pub const SEED_COMMAND: &str = "sim.seed";
/// Command that resolves one actor action.
pub const ACT_COMMAND: &str = "sim.act";

/// Parses one canonical `<name>@<version>` key.
///
/// # Errors
///
/// Panics only for the constants defined in this module, which are valid by construction.
fn key(name: &str, version: u32) -> SchemaKey {
    let id = SchemaId::parse(name).expect("package schema name must be canonical");
    let version = SchemaVersion::new(version).expect("schema version must be positive");
    SchemaKey::new(id, version)
}

/// Key of the world-state property.
pub fn state_key() -> SchemaKey {
    key(STATE_PROPERTY, 1)
}

/// Key of the actor property.
pub fn actor_key() -> SchemaKey {
    key(ACTOR_PROPERTY, 1)
}

/// Key of the seed command schema.
pub fn seed_key() -> SchemaKey {
    key(SEED_COMMAND, 1)
}

/// Key of the act command schema.
pub fn act_key() -> SchemaKey {
    key(ACT_COMMAND, 1)
}

/// Property definitions persisted into the world vocabulary on first activation.
pub fn property_definitions(owner: &ExtensionId) -> Vec<PropertyDefinition> {
    vec![
        PropertyDefinition::new(
            state_key(),
            owner.clone(),
            json!({
                "type": "object",
                "properties": {
                    "stability": { "type": "integer", "minimum": 0, "maximum": 10 },
                    "ruler": { "type": ["string", "null"] },
                    "rules": { "enum": ["loose", "strict"] }
                },
                "required": ["stability", "ruler"],
                "additionalProperties": false
            }),
        ),
        PropertyDefinition::new(
            actor_key(),
            owner.clone(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "minLength": 1, "maxLength": 64 },
                    "power": { "type": "integer", "minimum": 0 },
                    "alive": { "type": "boolean" },
                    "loyal": { "type": "boolean" }
                },
                "required": ["name", "power", "alive", "loyal"],
                "additionalProperties": false
            }),
        ),
    ]
}

/// Transient runtime command schemas registered with the world runtime.
pub fn runtime_schemas(owner: &ExtensionId) -> Vec<RuntimeSchemaDefinition> {
    vec![
        RuntimeSchemaDefinition::new(
            seed_key(),
            SchemaKind::Command,
            owner.clone(),
            json!({
                "type": "object",
                "properties": {
                    "state": { "type": "string" },
                    "stability": { "type": "integer", "minimum": 0, "maximum": 10 },
                    "rules": { "enum": ["loose", "strict"] },
                    "actors": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 8,
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string", "minLength": 1, "maxLength": 64 },
                                "power": { "type": "integer", "minimum": 0 },
                                "loyal": { "type": "boolean" }
                            },
                            "required": ["id", "name", "power", "loyal"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["state", "stability", "actors"],
                "additionalProperties": false
            }),
        ),
        RuntimeSchemaDefinition::new(
            act_key(),
            SchemaKind::Command,
            owner.clone(),
            json!({
                "type": "object",
                "properties": {
                    "state": { "type": "string" },
                    "actor": { "type": "string" },
                    "action": {
                        "enum": ["grab-power", "purge", "share", "hoard"]
                    },
                    "target": { "type": ["string", "null"] }
                },
                "required": ["state", "actor", "action"],
                "additionalProperties": false
            }),
        ),
    ]
}
