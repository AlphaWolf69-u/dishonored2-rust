//! Original particle/filing_container mesh-material parameters.
//! The shader fills authored container geometry; no liquid mesh is invented.
pub const IMAGE_NAMES: [&str; 6] = [
    "fx/diffuse",
    "fx/disto1",
    "fx/disto2",
    "fx/lookup_fire",
    "particlefresnelnormalmap",
    "particlefresnellut",
];
pub const IMAGE_DEFAULTS: [&str; 6] = [
    "_default",
    "_default",
    "_default",
    "_default",
    "ipr_constantcolor(0.5,0.5,0,0)",
    "_default",
];
pub const PARAMETER_NAMES: [&str; 18] = [
    "disto1_panningspeed",
    "disto2_panningspeed",
    "disto1_tilling",
    "disto2_tilling",
    "disto1_rotationspeed",
    "disto2_rotationspeed",
    "material_seed",
    "liquid_level",
    "liquid_threshold",
    "liquid_outer_alpha",
    "liquid_blur",
    "liquid_color_lerp",
    "liquid_color",
    "color_adjust",
    "alpha_adjust",
    "particlefresnelthreshold",
    "particlefresnelstrength",
    "particlealphatest",
];
pub const CLIP_PARAMETER_NAMES: [&str; 2] = ["fx/clipplane", "fx/clipplanefadedistance"];
pub const FADE_PARAMETER_NAMES: [&str; 2] = ["particledistancefadein", "particledistancefadeout"];
pub fn parameter_vectors(v: [[f32; 4]; 18], clip: [[f32; 4]; 2]) -> [[f32; 4]; 11] {
    [
        v[0],
        v[1],
        v[2],
        v[3],
        [v[4][0], v[5][0], v[6][0], v[7][0]],
        [v[8][0], v[9][0], v[10][0], v[11][0]],
        v[12],
        v[13],
        [v[14][2], v[15][0], v[16][0], v[17][0]],
        clip[0],
        [clip[1][0], 0., 0., 0.],
    ]
}
