#define_import_path dh2_indirect

@group(#{MATERIAL_BIND_GROUP}) @binding(100)
var<storage, read> probe_sh: array<array<vec4<f32>, 9>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(119)
var<storage, read> probe_words: array<u32>;

struct PackedIrradiance {
    luminance: f32,
    chroma_a: vec4<f32>,
    chroma_b: vec4<f32>,
}

fn game_vector(v: vec3<f32>) -> vec3<f32> { return vec3(v.x, -v.z, v.y); }

// indirectLighting/common: no saturation of tetrahedral barycentrics.
fn native_vertex_sh(tag: u32, vertex_index: u32, position: vec3<f32>) -> array<vec3<f32>, 9> {
    var sh: array<vec3<f32>, 9>;
    if (tag & 0x80000000u) == 0u {
        for (var i = 0u; i < 9u; i++) { sh[i] = probe_sh[tag][i].rgb; }
        return sh;
    }
    let instance = tag & 0x7fffffffu;
    let reference = probe_words[probe_words[0] + instance] + vertex_index;
    let tetra = probe_words[probe_words[1] + reference];
    let base = probe_words[2] + 6u * tetra;
    var positions: array<vec3<f32>, 4>;
    var weights: array<vec4<f32>, 4>;
    for (var i = 0u; i < 4u; i++) {
        let v = probe_words[3] + 4u * probe_words[base + i];
        positions[i] = bitcast<vec3<f32>>(vec3(probe_words[v], probe_words[v+1u], probe_words[v+2u]));
        let packed = probe_words[v+3u];
        weights[i] = vec4<f32>(vec4<u32>(packed & 255u, (packed >> 8u) & 255u, (packed >> 16u) & 255u, packed >> 24u)) * (1.0 / 255.0);
    }
    let d = game_vector(position) - positions[3];
    let v0 = positions[0] - positions[3];
    let v1 = positions[1] - positions[3];
    let v2 = positions[2] - positions[3];
    let determinant = dot(v0, cross(v1, v2));
    var reciprocal = 0.0;
    if abs(determinant) > 0.0 { reciprocal = 1.0 / determinant; }
    let b0 = reciprocal * dot(d, cross(v1, v2));
    let b1 = reciprocal * dot(d, cross(v2, v0));
    let b2 = reciprocal * dot(d, cross(v0, v1));
    let bary = vec4(b0, b1, b2, 1.0 - b0 - b1 - b2);
    let w = bary.x*weights[0] + bary.y*weights[1] + bary.z*weights[2] + bary.w*weights[3];
    let packed0 = probe_words[base+4u];
    let packed1 = probe_words[base+5u];
    let probes = vec4(packed0 & 65535u, packed0 >> 16u, packed1 & 65535u, packed1 >> 16u);
    for (var i = 0u; i < 9u; i++) {
        sh[i] = w.x*probe_sh[probes.x][i].rgb + w.y*probe_sh[probes.y][i].rgb
            + w.z*probe_sh[probes.z][i].rgb + w.w*probe_sh[probes.w][i].rgb;
    }
    return sh;
}

fn irradiance(d: vec3<f32>, sh: array<vec3<f32>, 9>) -> vec3<f32> {
    let x = d.x; let y = d.y; let z = d.z;
    let c1 = 0.4290427654048917; let c2 = 0.5116633539732442;
    let c3 = 0.2477079561003757; let c4 = 0.886226925452758;
    return max(vec3(0.0), c1*(x*x-y*y)*sh[8] + c3*(3.0*z*z-1.0)*sh[6] + c4*sh[0]
        + 2.0*c1*(sh[4]*x*y + sh[7]*x*z + sh[5]*y*z)
        + 2.0*c2*(sh[3]*x + sh[1]*y + sh[2]*z));
}

fn rgb_to_ycocg(rgb: vec3<f32>) -> vec3<f32> {
    let co = rgb.r - rgb.b;
    let t = rgb.b + co*0.5;
    let cg = rgb.g - t;
    return vec3(t + cg*0.5, co, cg);
}
fn ycocg_to_rgb(value: vec3<f32>) -> vec3<f32> {
    let t = value.x - value.z*0.5;
    let b = t - value.y*0.5;
    return vec3(value.y+b, value.z+t, b);
}

// PackIrradianceYCgCo is also present in cached native vertex DXBC:
// native-vertex-shaders/00037-00121d00.asm exports CoCg to o1/o2 and L to o7.z.
fn pack_vertex_irradiance(sh: array<vec3<f32>, 9>, tangent: vec3<f32>, bitangent: vec3<f32>, normal: vec3<f32>, two_sided: bool) -> PackedIrradiance {
    let t = game_vector(tangent); let b = game_vector(bitangent); let n = game_vector(normal);
    var values: array<vec3<f32>, 4>;
    values[0] = rgb_to_ycocg(irradiance(0.7071*t + 0.7071*n, sh));
    values[1] = rgb_to_ycocg(irradiance(-0.35355*t - 0.61237*b + 0.7071*n, sh));
    values[2] = rgb_to_ycocg(irradiance(-0.35355*t + 0.61237*b + 0.7071*n, sh));
    values[3] = rgb_to_ycocg(irradiance(select(n, -n, two_sided), sh));
    let luminance = max(max(values[0].x, values[1].x), max(values[2].x, values[3].x));
    let factor = 1.0 / max(1e-4, luminance*luminance);
    let c0 = values[0].yz*(factor*values[0].x);
    let c1 = values[1].yz*(factor*values[1].x);
    let c2 = values[2].yz*(factor*values[2].x);
    let c3 = values[3].yz*(factor*values[3].x);
    return PackedIrradiance(luminance, vec4(c0,c1), vec4(c2,c3));
}

fn unpack_diffuse(packed: PackedIrradiance, direction: vec3<f32>, tangent: vec3<f32>, bitangent: vec3<f32>, normal: vec3<f32>, ao: f32, two_sided: bool) -> vec3<f32> {
    let n = normal;
    var d = direction;
    if !two_sided {
        d = mix(n, d, ao);
        let length_d = length(d);
        if length_d > 1e-6 { d /= length_d; }
    }
    let ts = vec3(dot(d,tangent), dot(d,bitangent), dot(d,n));
    var weights = vec4(dot(ts,vec3(0.7071,0.0,0.7071)),
        dot(ts,vec3(-0.35355,-0.61237,0.7071)),
        dot(ts,vec3(-0.35355,0.61237,0.7071)), ts.z);
    if two_sided {
        weights.w = -ts.z;
        weights = 0.5 * clamp(weights, vec4(0.0), vec4(1.0));
    } else { weights *= 0.5; weights.w *= 1e-6; }
    let l = packed.luminance;
    let irradiance0 = ycocg_to_rgb(vec3(l,packed.chroma_a.xy*l));
    let irradiance1 = ycocg_to_rgb(vec3(l,packed.chroma_a.zw*l));
    let irradiance2 = ycocg_to_rgb(vec3(l,packed.chroma_b.xy*l));
    let irradiance3 = ycocg_to_rgb(vec3(l,packed.chroma_b.zw*l));
    return ao*(weights.x*irradiance0 + weights.y*irradiance1 + weights.z*irradiance2 + weights.w*irradiance3);
}
