//! arkOcean grid/patch vertex preparation and depth motion-vector equations.
//! Actual draw submission, current/previous wave selection and view admission
//! belong to the ocean pass owner; no camera/history defaults are inferred here.

pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_ocean_noise.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_surface.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_motion_vector.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_vertex.wgsl"),
);

#[derive(Clone, Copy, Debug)]
pub struct OceanVertexView {
    pub clip_from_world: [f32; 16],
    pub previous_clip_from_world: [f32; 16],
    /// Current game viewMatrix[2], not a coordinate-converted camera row.
    pub view_row: [f32; 4],
    pub jitter_relative_to_previous: [f32; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct OceanVertexParameters {
    pub view: OceanVertexView,
    /// ocean_patch_transform's game-space origin.xyz and grid spacing.
    pub patch_transform: [f32; 4],
    /// Resolved ocean/PerlinBlendEnd; BlendStart is in surface parameters.
    pub perlin_blend_end: f32,
}
impl OceanVertexParameters {
    pub fn uniform_bytes(&self) -> [u8; 176] {
        let mut bytes = [0; 176];
        let tail = [
            self.perlin_blend_end,
            self.view.jitter_relative_to_previous[0],
            self.view.jitter_relative_to_previous[1],
            0.,
        ];
        let values = self
            .view
            .clip_from_world
            .into_iter()
            .chain(self.view.previous_clip_from_world)
            .chain(self.view.view_row)
            .chain(self.patch_transform)
            .chain(tail);
        for (i, value) in values.enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
#[path = "ocean_vertex/tests.rs"]
mod tests;
