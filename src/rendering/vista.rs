//! Baked-lightmap vista shading contracts from the installed arkVista program.
//! These meshes use UV1 lightmaps and their alpha for sun visibility; they do
//! not use ordinary mesh probes, local lights, or cascaded sun-shadow visibility.

pub const PARAMETER_NAMES: [&str; 13] = [
    "lightmapluminance",
    "arkvista/glossboost/lightangle_start",
    "arkvista/glossboost/lightangle_end",
    "arkvista/glossboost/glossmap_start",
    "arkvista/glossboost/glossmap_end",
    "arkvista/glossboost/lightmapshadow_start",
    "arkvista/glossboost/lightmapshadow_end",
    "arkvista/glossboost/target_gloss",
    "arkvista/glossboost/metal_min",
    "arkvista/glossboost/metal_max",
    "arkvista/glossboost/viewdependantfactor",
    "arkvista/glossboost/viewdependantexponent",
    "emissiveintensity",
];

/// Pack named material/registered declaration values for the WGSL transport.
/// The resolver must supply actual registered defaults, not a shader-family guess.
pub fn parameter_vectors(
    mut resolve: impl FnMut(&str) -> Result<[f32; 4], String>,
) -> Result<[[f32; 4]; 4], String> {
    let mut v = [[0.; 4]; 13];
    for (i, name) in PARAMETER_NAMES.iter().enumerate() {
        v[i] = resolve(name)?;
    }
    Ok([
        v[0],
        [v[1][0], v[2][0], v[3][0], v[4][0]],
        [v[5][0], v[6][0], v[7][0], v[8][0]],
        [v[9][0], v[10][0], v[11][0], v[12][0]],
    ])
}
