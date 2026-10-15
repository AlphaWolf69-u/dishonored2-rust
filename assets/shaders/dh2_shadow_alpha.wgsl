// arkDefaultHelpers::ClipZorShadowPass: shadow alpha tests raw diffuse alpha
// with the owner's bilinear sampler against 0.5, NOT forward DefaultAlphaTest
// or the layered/tinted albedo. Rigid vertices only; cloth/wind are separate.
// Cached arkdefault VS 42db1831-436080c4-98198e41-af225f13 passes RAW
// texcoords. PS 0d07eb8b-16bf4f4c-3886484d-6ad44f16 applies scale/bias
// after interpolation with MAD; doing it per vertex changes rounding/LOD.
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
// The owner selects Layer0_UVSet's actual mesh stream at location 1 and
// provides the same authored scale/bias used to initialize layerTexCoords[0].
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var bilinear_sampler: sampler;
@group(1) @binding(2) var<uniform> uv_scale_bias: vec4<f32>;

struct VertexInput {
    @location(0) vertex_position: vec3<f32>,
    @location(1) layer0_uv: vec2<f32>,
}
struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) layer0_uv: vec2<f32>,
}
@vertex
fn vertex(input: VertexInput) -> VertexOutput {
    let position = vec4<f32>(input.vertex_position, 1.0);
    var output: VertexOutput;
    output.clip.z = dot(per_instance.cb_mvpmatrixz, position);
    output.clip.z *= per_instance.cb_modeldepthhack;
    output.clip.x = dot(per_instance.cb_mvpmatrixx, position);
    output.clip.y = dot(per_instance.cb_mvpmatrixy, position);
    output.clip.w = dot(per_instance.cb_mvpmatrixw, position);
    output.layer0_uv = input.layer0_uv;
    return output;
}
@fragment
fn fragment(input: VertexOutput) {
    let uv = fma(input.layer0_uv, uv_scale_bias.xy, uv_scale_bias.zw);
    if textureSample(diffuse, bilinear_sampler, uv).a < 0.5 {
        discard;
    }
}
