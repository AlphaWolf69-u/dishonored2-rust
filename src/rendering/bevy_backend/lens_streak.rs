//! Installed final-display lens streak. The frame supplies source view-axis
//! and authored radius/offset/width/opacity; no artificial flare is generated.
pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_lens_streak.wgsl");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires GPU; compares installed lens-streak calculations"]
    fn lens_streak_matches_installed_shader() {
        let source = format!(
            "{SHADER_SOURCE}\n{}",
            r#"
@group(0) @binding(0) var<uniform> options:vec4<u32>;
struct Query {uv_axis:vec4<f32>,settings:vec4<f32>,}
@group(0) @binding(3) var<storage,read> queries:array<Query>;
@group(0) @binding(4) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x+options.x];let p=lens_rotate_scaled(q.uv_axis.xy-vec2(0.5),q.uv_axis.z);
    let slice=lens_slice_phase(p).x;
    output[id.x]=vec4(p,lens_streak(q.uv_axis.xy,q.uv_axis.z,q.settings),lens_random(slice));
}
"#
        );
        assert_eq!(
            super::super::test_water_inputs::compare_storage_inputs(
                &source,
                include_str!("../../../tests/fixtures/lens_streak.txt"),
                8,
                4,
                &[]
            ),
            1
        );
    }
}
