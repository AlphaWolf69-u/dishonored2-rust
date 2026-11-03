#define_import_path dh2_decal_projection

// Host instance records reference shared meshes and the current skin palette.
// The original WORLD projection payload is the three world-to-decal rows and
// colour (29a270); the extra fields replace separate host draw parameters.
struct GpuProjection {
    world_to_decal: array<vec4<f32>, 3>,
    colour: vec4<f32>,
    lighting: array<vec4<f32>, 9>,
}
struct GpuReceiver { model: array<vec4<f32>, 3>, indices: vec4<u32> }
struct DecalProjectionInstance { receiver: GpuReceiver, projection: GpuProjection }
@group(4) @binding(0) var<storage, read> decal_projections: array<GpuProjection>;
@group(4) @binding(1) var<storage, read> decal_receivers: array<GpuReceiver>;
@group(4) @binding(2) var<storage, read> decal_remap: array<u32>;
@group(4) @binding(3) var<storage, read> decal_joints: array<mat4x4<f32>>;
@group(4) @binding(4) var decal_bilinear: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(175) var decal_opacity: texture_2d<f32>;
struct ProjectionParameters { colour_adjust: vec4<f32>, box_projection: vec4<f32>, parameters: vec4<f32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(176) var<uniform> decal_parameters: ProjectionParameters;

struct DecalProjectionDraw { mesh_index: u32, first: u32 }
var<immediate> decal_draw: DecalProjectionDraw;
fn decal_record(index: u32) -> DecalProjectionInstance {
    let pair = decal_remap[index];
    return DecalProjectionInstance(decal_receivers[pair & 0x00ffffffu], decal_projections[pair >> 24u]);
}
fn decal_instance(index: u32) -> DecalProjectionInstance { return decal_record(decal_draw.first + index); }
fn decal_world_from_local(instance: DecalProjectionInstance) -> mat4x4<f32> {
    return transpose(mat4x4(instance.receiver.model[0], instance.receiver.model[1], instance.receiver.model[2], vec4(0.0, 0.0, 0.0, 1.0)));
}
fn decal_position(instance: DecalProjectionInstance, world: vec3<f32>) -> vec3<f32> {
    let p = vec4(world, 1.0);
    return vec3(dot(instance.projection.world_to_decal[0], p), dot(instance.projection.world_to_decal[1], p), dot(instance.projection.world_to_decal[2], p));
}
fn decal_inside(position: vec3<f32>) -> bool {
    return all(position >= vec3(-1.0)) && all(position <= vec3(1.0, 1.0, 0.0));
}
// HLSL smoothstep permits the descending edges authored in decalBoxProjection.
// Spell out its polynomial rather than relying on WGSL's undefined edge order.
fn decal_smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let t = clamp((value - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
fn decal_min_diff(centre: vec3<f32>, positive: vec3<f32>, negative: vec3<f32>) -> vec3<f32> {
    let a = positive - centre;
    let b = centre - negative;
    return select(b, a, dot(a, a) < dot(b, b));
}
struct DecalCoordinates { uv: vec2<f32>, alpha: f32 }
fn decal_depth_coordinates(instance: DecalProjectionInstance, position: vec3<f32>, normal: vec3<f32>) -> DecalCoordinates {
    var alpha = abs(position.z * 2.0) * decal_smoothstep(-0.9, 1.0,
        dot(normal, instance.projection.world_to_decal[2].xyz));
    let ray = -position;
    let a = dot(ray, ray);
    let b = 2.0 * dot(ray, position);
    let c = dot(position, position) - decal_parameters.box_projection.x;
    let discriminant = b * b - 4.0 * a * c;
    var hit = position.xy;
    if discriminant >= 0.0 {
        // Installed PS SolveQuadratic: stable q, then the smaller root.
        let q = -0.5 * (b + sign(b) * sqrt(discriminant));
        let root = min(q / a, c / q);
        hit = position.xy + ray.xy * root;
        alpha *= decal_smoothstep(1.0, decal_parameters.box_projection.y, dot(position, position));
    }
    return DecalCoordinates(hit * 0.5 + 0.5, max(alpha, 0.01569));
}
fn decal_effect_colour(instance: DecalProjectionInstance, uv: vec2<f32>, diffuse: vec4<f32>) -> vec4<f32> {
#ifdef DH2_DECAL_FX
    var result = diffuse * decal_parameters.colour_adjust;
    let opacity = textureSample(decal_opacity, decal_bilinear, uv).r;
    let fade = clamp(opacity * result.a - instance.projection.colour.x, 0.0, 1.0) * result.a;
    result.a *= decal_smoothstep(instance.projection.colour.y, instance.projection.colour.y + decal_parameters.parameters.x, fade) * instance.projection.colour.w;
    return result;
#else
#ifdef DH2_DECAL_NONE
    return diffuse;
#else
    return diffuse * instance.projection.colour;
#endif
#endif
}

fn decal_skin4(joints: vec4<u32>, weights: vec4<f32>, base: u32) -> mat4x4<f32> {
    return weights.x * decal_joints[base + joints.x] + weights.y * decal_joints[base + joints.y]
        + weights.z * decal_joints[base + joints.z] + weights.w * decal_joints[base + joints.w];
}
fn decal_skin_term(m: mat4x4<f32>, weight: f32, accumulator: mat4x4<f32>) -> mat4x4<f32> {
    return mat4x4(fma(m[0], vec4(weight), accumulator[0]), fma(m[1], vec4(weight), accumulator[1]),
        fma(m[2], vec4(weight), accumulator[2]), fma(m[3], vec4(weight), accumulator[3]));
}
fn decal_skin8(a: vec4<u32>, b: vec4<u32>, wa: vec4<f32>, wb: vec4<f32>, base: u32) -> mat4x4<f32> {
    var m = decal_joints[base + a.y] * wa.y;
    m = decal_skin_term(decal_joints[base + a.x], wa.x, m);
    m = decal_skin_term(decal_joints[base + a.z], wa.z, m);
    m = decal_skin_term(decal_joints[base + a.w], wa.w, m);
    m = decal_skin_term(decal_joints[base + b.x], wb.x, m);
    m = decal_skin_term(decal_joints[base + b.y], wb.y, m);
    m = decal_skin_term(decal_joints[base + b.z], wb.z, m);
    return decal_skin_term(decal_joints[base + b.w], wb.w, m);
}

// Decal_FetchDiffuseAlbedo's DEPTH_BOX SampleGrad arguments are scalar-splat
// coarse derivatives of depth-reconstructed dPdu/dPdv, not projected UVs.
var<private> decal_diffuse_dx: vec2<f32>;
var<private> decal_diffuse_dy: vec2<f32>;
fn decal_set_gradients(dx: vec3<f32>, dy: vec3<f32>) {
    decal_diffuse_dx = vec2(dx.x); decal_diffuse_dy = vec2(dy.x);
}
