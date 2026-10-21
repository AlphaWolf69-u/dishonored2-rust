//! PC vertex/fragment probe transport. Probe selection and array remapping
//! belong to the actual vertex/instance producer, rather than this decoder.
pub const INPUT_SOURCE: &str = include_str!("../../../assets/shaders/dh2_water_probes.wgsl");
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_probe_weights.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_probes.wgsl"),
);

#[cfg(test)]
#[path = "water_probes/tests.rs"]
mod tests;
