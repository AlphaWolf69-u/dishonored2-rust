//! Authored sky and cloud parameters from arkSky/arkClouds declarations.
pub const PARAMETER_NAMES: [&str; 11] = [
    "skycoef",
    "skystarsize",
    "skystardensity",
    "skystarblink",
    "skystarintensity",
    "cloudspeed",
    "cloudverticalalpha",
    "clouddensitycoef",
    "cloudverticalextinction",
    "cloudbacklitcoef",
    "cloudambientextinction",
];
pub fn parameter_vectors(v: [[f32; 4]; 11]) -> [[f32; 4]; 5] {
    [
        v[0],
        [v[1][0], v[1][1], v[2][0], v[3][0]],
        [v[4][0], v[5][0], v[6][0], v[7][0]],
        [v[8][0], v[9][0], v[9][1], v[10][0]],
        [0.; 4],
    ]
}
