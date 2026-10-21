//! Source water refraction, caustics and low-quality directional shadow helpers.
//! The caller supplies effective material, exposure and same-view depth inputs.
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_texture_array_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_refraction.wgsl"),
);

#[derive(Clone, Copy, Debug)]
pub struct WaterRefractionParameters {
    /// localTime.x, CausticsSpeed, CausticsTiling, CausticsChromaticAberration.
    pub clock_caustics: [f32; 4],
    /// resolutionScale.xy, positionToViewTexture.zw.
    pub screen: [f32; 4],
    /// -inverseProjection[2].w, inverseProjection[3][2], [3][3], Refraction.
    pub unproject: [f32; 4],
    /// wsRefractedLightDir.xy, CausticsFade, actual WaterLevel.
    pub light_level: [f32; 4],
    /// ExtinctionCoeff.xyz, useCompressedHdrBuffers.
    pub extinction: [f32; 4],
    /// Actual auto-exposure EngineLuminanceFactor, view_white_level.x, unused.
    pub hdr: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct WaterShadowParameters {
    /// Game's ROWS encoded as WGSL columns, as in the shared shadow transport.
    pub world_to_shadow: [[f32; 16]; 5],
    pub view_row: [f32; 4],
    pub slice_fars: [f32; 4],
    pub sun_direction: [f32; 4],
    pub shadow_size: [f32; 4],
    /// normalOffset, bias, slopeScaleBias, unused.
    pub bias: [f32; 4],
    /// hasParallelShadow, shadowCommon normal/slope/bias/post-projection bits.
    /// UV-normal-offset is not supported by the shared shadow shader transport.
    pub options: [u32; 4],
}

/// The two explicit shader blocks, with no camera/history/exposure defaults.
pub fn uniform_bytes(
    water: &WaterRefractionParameters,
    shadow: &WaterShadowParameters,
) -> [u8; 512] {
    let mut bytes = [0; 512];
    let values = [
        water.clock_caustics,
        water.screen,
        water.unproject,
        water.light_level,
        water.extinction,
        water.hdr,
    ]
    .into_iter()
    .flatten()
    .chain(shadow.world_to_shadow.into_iter().flatten())
    .chain(
        [
            shadow.view_row,
            shadow.slice_fars,
            shadow.sun_direction,
            shadow.shadow_size,
            shadow.bias,
        ]
        .into_iter()
        .flatten(),
    );
    for (i, value) in values.enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, value) in shadow.options.into_iter().enumerate() {
        bytes[496 + i * 4..500 + i * 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
#[path = "water_refraction/tests.rs"]
mod tests;
