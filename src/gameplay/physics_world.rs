//! Physics-world configuration and owned runtime composition.
mod info;
pub use info::{WorldHandles, WorldInfo};
mod publication;
pub use publication::{BodyAddition, BodyPublication, PublicationCallbacks, TransformCallbacks};
mod context;
pub use context::{PublicationContext, PublicationEvents, PublicationGeometry};
mod events;
pub use events::{ComponentPublicationEvents, PublicationEvent, PublicationSignals};
pub(crate) use events::BodyEventCallback;
mod motion_creation;
pub use motion_creation::MotionCreationCallbacks;
pub(crate) mod body_creation;
pub(crate) mod storage;
#[cfg(test)]
#[path = "physics_world/publication_tests.rs"]
mod publication_tests;
pub mod update_queue;
pub mod command_buffer;
pub mod commands;
