//! Physics-world configuration and owned runtime composition.
pub(crate) mod thread_policy;
mod info;
pub use info::{WorldHandles, WorldInfo};
mod publication;
pub use publication::{BodyAddition, BodyPublication, PublicationCallbacks, TransformCallbacks};
mod context;
pub use context::{PublicationContext, PublicationEvents, PublicationGeometry};
mod events;
pub(crate) use events::BodyEventCallback;
pub use events::{ComponentPublicationEvents, ConstraintChange, PublicationEvent, PublicationSignals};
mod motion_creation;
pub use motion_creation::MotionCreationCallbacks;
pub(crate) mod body_activation_events;
pub(crate) mod body_creation;
pub mod command_buffer;
pub mod commands;
#[cfg(test)]
#[path = "physics_world/publication_tests.rs"]
mod publication_tests;
pub(crate) mod storage;
pub mod update_queue;

pub(crate) mod contacts;
pub(crate) mod contact_geometry;

pub(crate) mod contact_modes;
pub(crate) mod trigger_events;
