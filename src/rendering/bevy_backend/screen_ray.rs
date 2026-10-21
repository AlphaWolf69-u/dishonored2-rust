//! Hierarchical screen-space ray traversal over the actual scene depth pyramid.
pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_screen_ray.wgsl");

#[derive(Clone, Copy, Debug)]
pub struct ScreenRayParameters {
    pub camera_to_clip: [f32; 16],
    /// -inverseProjection[2].w, inverseProjection[3][2], [3][3], unused.
    pub unproject: [f32; 4],
    /// Effective full-resolution extent, followed by unused words.
    pub extent: [u32; 4],
}
impl ScreenRayParameters {
    pub fn uniform_bytes(&self) -> [u8; 96] {
        let mut bytes = [0; 96];
        for (i, value) in self
            .camera_to_clip
            .into_iter()
            .chain(self.unproject)
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (i, value) in self.extent.into_iter().enumerate() {
            bytes[80 + i * 4..84 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
#[path = "screen_ray/tests.rs"]
mod tests;
