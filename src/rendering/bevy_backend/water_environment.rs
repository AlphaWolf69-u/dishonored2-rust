//! Water reflection's authored sky/probe environment selection and sampling.
pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_water_environment.wgsl");

#[cfg(test)]
#[path = "water_environment/tests.rs"]
mod tests;
