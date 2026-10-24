//! Forward/Z arkvegetation AnimateVegetation in the Bevy mesh vertex programs.
//! The installed forward VS (674e25d0 family) bends with the same arithmetic
//! as the shadow VS; per-instance rows, material vegetAnim vectors, the view
//! `$localTime` and the windInfo record arrive through two shared buffers.
use super::shadow_pass::{VEGETATION_COLOR, VEGETATION_NORMAL, VEGETATION_POSITION};
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    prelude::*,
    render::render_resource::{RenderPipelineDescriptor, SpecializedMeshPipelineError},
    shader::Shader,
};

const ANIMATE_VEGETATION_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("3d3c6a42-9c55-4b8e-a1d6-0f62a8c7e915");
const VEGETATION_WIND_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("a7f0e2b1-5d84-4c6f-9e3a-71b2c4d8f063");

/// `VegetationInstance` in dh2_vegetation_wind.wgsl: seven float4/uint4 rows.
pub const INSTANCE_WORDS: usize = 28;
/// The slot table's "no record" value.
pub const NO_SLOT: u32 = u32::MAX;

pub fn register_shaders(app: &mut App) {
    // Shadow programs concatenate this file; materials import it.
    bevy::asset::load_internal_asset!(
        app,
        ANIMATE_VEGETATION_SHADER,
        "../../../assets/shaders/dh2_animate_vegetation.wgsl",
        |source: &str, path| Shader::from_wgsl(
            format!("#define_import_path dh2_animate_vegetation\n{source}"),
            path
        )
    );
    bevy::asset::load_internal_asset!(
        app,
        VEGETATION_WIND_SHADER,
        "../../../assets/shaders/dh2_vegetation_wind.wgsl",
        Shader::from_wgsl
    );
}

/// One instance record. `rows` are the game-space model rows; `vectors` are
/// the material's vegetAnim MainBendingVar, MainBendingFreq,
/// MainBendingAmplitude, LeafMinWindStrength, LeafFreq, LeafAmplitude and
/// BranchAmplitude; `wind` is windInfo.wind when the instance reads it.
/// `rows: None` places an unbaked model through its current mesh transform.
pub fn instance_words(
    rows: Option<[[f32; 4]; 3]>,
    vectors: [f32; 7],
    local_time: f32,
    wind: Option<[f32; 4]>,
) -> [u32; INSTANCE_WORDS] {
    let mut words = [0; INSTANCE_WORDS];
    let floats = rows
        .unwrap_or([[0.0; 4]; 3])
        .into_iter()
        .flatten()
        .chain(vectors)
        .chain([local_time])
        .chain(wind.unwrap_or([0.0; 4]));
    for (word, value) in words.iter_mut().zip(floats) {
        *word = value.to_bits();
    }
    words[24] = u32::from(wind.is_some());
    words[25] = u32::from(rows.is_none());
    words
}

/// arkVegetation (program 2) pipelines whose mesh retains the original
/// local channels bend in both the forward and depth prepass vertex stages.
pub fn specialize(
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
    program: u32,
) -> Result<(), SpecializedMeshPipelineError> {
    if program != 2
        || !(layout.0.contains(VEGETATION_POSITION)
            && layout.0.contains(VEGETATION_NORMAL)
            && layout.0.contains(VEGETATION_COLOR))
    {
        return Ok(());
    }
    let extra = layout.0.get_layout(&[
        VEGETATION_POSITION.at_shader_location(10),
        VEGETATION_NORMAL.at_shader_location(11),
        VEGETATION_COLOR.at_shader_location(12),
    ])?;
    descriptor.vertex.buffers[0]
        .attributes
        .extend(extra.attributes);
    descriptor
        .vertex
        .shader_defs
        .push("DH2_VEGETATION_WIND".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_words_match_the_wgsl_record_layout() {
        let rows = [[1., 2., 3., 4.], [5., 6., 7., 8.], [9., 10., 11., 12.]];
        let vectors = [13., 14., 15., 16., 17., 18., 19.];
        let fixed = instance_words(Some(rows), vectors, 20., None);
        let floats: Vec<f32> = fixed[..24].iter().map(|w| f32::from_bits(*w)).collect();
        assert_eq!(floats[..20], (1..=20).map(|v| v as f32).collect::<Vec<_>>()[..]);
        assert_eq!(&fixed[20..], &[0; 8]);
        let wind = instance_words(Some(rows), vectors, 20., Some([0.6, 0., 0.8, 3.]));
        assert_eq!(f32::from_bits(wind[23]), 3.);
        assert_eq!(&wind[24..], &[1, 0, 0, 0]);
        let moving = instance_words(None, vectors, 20., None);
        assert_eq!(&moving[..12], &[0; 12]);
        assert_eq!(&moving[24..], &[0, 1, 0, 0]);
    }
}
