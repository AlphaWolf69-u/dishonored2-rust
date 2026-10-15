// Captured native opaque model-shadow vertex program 17296 (hash
// be596c78-7b23f0b2-921af52f-29b1e602), RenderDoc event 40712.
// This is only the rigid opaque model path; alpha, skinned and paraboloid
// casters use different original programs and are intentionally not covered.
struct PerInstanceCB {
    cb_mvpmatrixw: vec4<f32>,
    cb_mvpmatrixz: vec4<f32>,
    cb_mvpmatrixy: vec4<f32>,
    cb_mvpmatrixx: vec4<f32>,
    cb_modeldepthhack: f32,
    padding0: f32,
    padding1: f32,
    padding2: f32,
}

@group(0) @binding(0) var<uniform> per_instance: PerInstanceCB;

struct VertexInput {
    @location(0) vertex_position: vec3<f32>,
}

@vertex
fn vertex(input: VertexInput) -> @builtin(position) vec4<f32> {
    let position = vec4<f32>(input.vertex_position, 1.0);
    var clip = vec4<f32>(0.0);
    clip.z = dot(per_instance.cb_mvpmatrixz, position);
    clip.z *= per_instance.cb_modeldepthhack;
    clip.x = dot(per_instance.cb_mvpmatrixx, position);
    clip.y = dot(per_instance.cb_mvpmatrixy, position);
    clip.w = dot(per_instance.cb_mvpmatrixw, position);
    return clip;
}
