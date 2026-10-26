//! Trigger pose publication (779af0), including its water-plane notification.
use crate::gameplay::{reference_frame::ReferenceFrame, shape_transform::ShapeMatrix};

/// The actual physics command owner supplies both immediate and queued paths.
/// A queued pose is command050 (size80), subtype60200, with activation byte0.
pub trait TriggerTransformWorld {
    fn deferred(&mut self) -> bool;
    fn body_id(&self) -> Result<u32, String>;
    fn set_transform(&mut self, body: u32, frame: ShapeMatrix) -> Result<(), String>;
    fn queue_transform(&mut self, body: u32, frame: ShapeMatrix) -> Result<(), String>;
    /// 18d020 receives two zero velocity vectors and a freshly read mode flag.
    fn reset_motion(&mut self, deferred: bool) -> Result<(), String>;
    /// Resolve the live water reference and body ID after physics callbacks,
    /// then use WaterComponent::update_plane with the body's main shape.
    fn update_water_plane(&mut self, frame: ShapeMatrix) -> Result<(), String>;
}

/// A deferred body move still updates the water plane immediately from the
/// requested pose. Reading the old body's transform here causes water-volume
/// selection to lag movement; the game passes the new local frame directly.
pub fn publish_transform(
    pose: ReferenceFrame,
    clear_motion: bool,
    world: &mut impl TriggerTransformWorld,
) -> Result<(), String> {
    let frame = [
        [pose.basis[0][0], pose.basis[0][1], pose.basis[0][2], 0.],
        [pose.basis[1][0], pose.basis[1][1], pose.basis[1][2], 0.],
        [pose.basis[2][0], pose.basis[2][1], pose.basis[2][2], 0.],
        [pose.position[0], pose.position[1], pose.position[2], 1.],
    ];
    let deferred = world.deferred();
    let body = world.body_id()?;
    if deferred {
        world.queue_transform(body, frame)?;
    } else {
        world.set_transform(body, frame)?;
    }
    if clear_motion {
        let deferred = world.deferred();
        world.reset_motion(deferred)?;
    }
    world.update_water_plane(frame)?;
    Ok(())
}

#[cfg(test)]
#[path = "transform_tests.rs"]
mod tests;
