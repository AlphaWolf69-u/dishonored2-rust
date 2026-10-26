//! arkCpntWater's retained owner and plane update (4d5b90/4d6a40).
use super::{
    collision_body::CollisionBody,
    component_state::{ComponentInstance, ComponentOwner},
    shape_transform::ShapeMatrix,
};
pub mod startup;

#[derive(Default)]
pub struct WaterComponent {
    pub owner: ComponentOwner,
    // 4d5b90 does not initialize the plane. Represent that state explicitly
    // instead of importing allocator bytes or inventing a zero-height surface.
    plane: Option<[f32; 4]>,
    /// The constructor leaves +30..3f unspecified as well. Start publishes the
    /// declaration's +7c/+80/+84/+78 words in that order.
    flow_and_density: Option<[f32; 4]>,
}
impl ComponentInstance for WaterComponent {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
impl WaterComponent {
    pub fn plane(&self) -> Option<[f32; 4]> {
        self.plane
    }
    pub fn flow_and_density(&self) -> Option<[f32; 4]> {
        self.flow_and_density
    }

    /// 4d6a40 reads the body's main shape, not a selected LOD shape. The caller
    /// supplies its actual virtual20 calcAabb using the passed transform.
    pub fn update_transform(
        &mut self,
        body: &CollisionBody,
        frame: ShapeMatrix,
        mut bounds: impl FnMut(usize, ShapeMatrix) -> Result<[[f32; 4]; 2], String>,
    ) -> Result<bool, String> {
        self.update_plane(|| bounds(body.shape, frame))
    }

    /// Keep phase admission ahead of the live body's shape lookup. The world
    /// may own both the body and its mutable shape-query implementation.
    pub(crate) fn update_plane(
        &mut self,
        mut bounds: impl FnMut() -> Result<[[f32; 4]; 2], String>,
    ) -> Result<bool, String> {
        if self.owner.flags & 0xc < 8 {
            return Ok(false);
        }
        let bounds = bounds()?;
        self.plane = Some([0., 0., 1., crate::float::subtract(0., bounds[1][2])]);
        Ok(true)
    }
}

#[cfg(test)]
#[path = "water_component/tests.rs"]
mod tests;
