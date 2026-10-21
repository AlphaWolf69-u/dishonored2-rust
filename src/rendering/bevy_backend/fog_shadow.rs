//! Source fog-shadow raymarch, including projected height and temporal branches.
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_fog_shadow.wgsl"),
);

#[derive(Clone, Copy, Debug)]
pub struct FogShadowMaterialOptions {
    pub projected: bool,
    pub temporal: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FogShadowParameters {
    pub viewport: [f32; 4],
    pub resolution_projected: [f32; 4],
    pub jitter: [f32; 4],
    pub options: [u32; 4],
}

impl FogShadowParameters {
    pub fn uniform_bytes(&self) -> [u8; 64] {
        let mut bytes = [0; 64];
        for (i, v) in self
            .viewport
            .into_iter()
            .chain(self.resolution_projected)
            .chain(self.jitter)
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, v) in self.options.into_iter().enumerate() {
            bytes[48 + i * 4..52 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
#[path = "fog_shadow/tests.rs"]
mod tests;
