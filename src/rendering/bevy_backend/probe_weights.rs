//! Shared GPU probe-weight interpolation for indirect lighting and water.
pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_probe_weights.wgsl");
pub fn indirect_source(source: &str) -> String {
    format!("{SHADER_SOURCE}\n{source}")
}
pub fn indirect_shader(source: &str, path: impl Into<String>) -> bevy::shader::Shader {
    bevy::shader::Shader::from_wgsl(indirect_source(source), path)
}
