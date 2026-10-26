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
struct VertexLighting {
    sh: array<vec3<f32>, 9>,
    cube_index: u32,
    reflection_gain: f32,
}
fn vertex_lighting(tag: u32, vertex_index: u32, position: vec3<f32>) -> VertexLighting {
    var sh: array<vec3<f32>, 9>;
    if (tag & 0x80000000u) == 0u {
        for (var i = 0u; i < 9u; i++) { sh[i] = probe_sh[tag][i].rgb; }
        return VertexLighting(sh,u32(max(0.0,probe_sh[tag][0].w)),probe_sh[tag][1].w);
    }
    let instance = tag & 0x7fffffffu;
    let reference = probe_words[probe_words[0] + instance] + vertex_index;
    let tetra = probe_words[probe_words[1] + reference];
    return tetrahedron_lighting_game(tetra,game_vector(position));
}

fn tetrahedron_lighting_game(tetra:u32,position:vec3<f32>)->VertexLighting {
    var sh: array<vec3<f32>, 9>;
    let base = probe_words[2] + 6u * tetra;
    var positions: array<vec3<f32>, 4>;
    var weights: array<vec4<f32>, 4>;
    for (var i = 0u; i < 4u; i++) {
        let v = probe_words[3] + 4u * probe_words[base + i];
        positions[i] = bitcast<vec3<f32>>(vec3(probe_words[v], probe_words[v+1u], probe_words[v+2u]));
        let packed = probe_words[v+3u];
        weights[i] = vec4<f32>(vec4<u32>(packed & 255u, (packed >> 8u) & 255u, (packed >> 16u) & 255u, packed >> 24u)) * (1.0 / 255.0);
    }
    let w = tetrahedron_probe_weights(position,positions,weights);
    let packed0 = probe_words[base+4u];
    let packed1 = probe_words[base+5u];
    let probes = vec4(packed0 & 65535u, packed0 >> 16u, packed1 & 65535u, packed1 >> 16u);
    for (var i = 0u; i < 9u; i++) {
        sh[i] = w.x*probe_sh[probes.x][i].rgb + w.y*probe_sh[probes.y][i].rgb
            + w.z*probe_sh[probes.z][i].rgb + w.w*probe_sh[probes.w][i].rgb;
    }
    var best_weight=w.x;
    var best_probe=probes.x;
    if w.y>best_weight {best_weight=w.y;best_probe=probes.y;}
    if w.z>best_weight {best_weight=w.z;best_probe=probes.z;}
    if w.w>best_weight {best_probe=probes.w;}
    let info=probe_words[8];
    let gains=vec4(bitcast<f32>(probe_words[info+2u*probes.x+1u]),
        bitcast<f32>(probe_words[info+2u*probes.y+1u]),
        bitcast<f32>(probe_words[info+2u*probes.z+1u]),
        bitcast<f32>(probe_words[info+2u*probes.w+1u]));
    return VertexLighting(sh,probe_words[info+2u*best_probe],max(0.0001,dot(w,gains)));
}

fn vertex_sh(tag: u32, vertex_index: u32, position: vec3<f32>) -> array<vec3<f32>, 9> {
    return vertex_lighting(tag,vertex_index,position).sh;
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
    return pack_vertex_irradiance_game(sh,game_vector(tangent),game_vector(bitangent),game_vector(normal),two_sided);
}
fn pack_vertex_irradiance_game(sh:array<vec3<f32>,9>,t:vec3<f32>,b:vec3<f32>,n:vec3<f32>,two_sided:bool)->PackedIrradiance {
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

// lighting::NOTOUCHY_MapGlossAndMetalToLightingPipelineValue +
// lightcontext::ComputeAmbientLighting, using fresnel's Ward diffuse integral.
// Probe samples are irradiance; this conversion produces reflected radiance.
struct MaterialLightingValues {
    diffuse:vec3<f32>,
    ior:vec3<f32>,
    reflectance:vec3<f32>,
    fresnel_strength:f32,
}
fn material_lighting_values(albedo:vec3<f32>,roughness:f32,metallic:f32,
    specular_tint:vec3<f32>,use_metallic_specular:bool,
    dielectric:vec3<f32>,metal_diffuse:vec3<f32>)->MaterialLightingValues {
    let gloss=1.0-roughness;
    var metal=metallic;var tint=specular_tint;
    if use_metallic_specular {
        metal*=metal;
        tint=mix(dielectric,albedo,metal);
        let low=smoothstep(0.0,1.07,gloss);
        let high=smoothstep(0.05,1.0,gloss);
        let mapped=mix(low,high,gloss);
        tint*=clamp((mapped+metal)/0.05,0.0,1.0);
    }
    let diffuse=mix(albedo,metal_diffuse,metal);
    let root=sqrt(tint);
    let ior=(vec3(1.0)+root)/(vec3(1.00001)-root);
    let ratio=(ior-vec3(1.0))/(ior+vec3(1.0));let f0=ratio*ratio;
    return MaterialLightingValues(diffuse,ior,f0,smoothstep(0.1,1.0,gloss));
}
fn evaluate_mapped_diffuse_response(material:MaterialLightingValues,roughness:f32)->vec3<f32> {
    let r=roughness;let r2=r*r;let r3=r2*r;
    let base=0.181446-0.133354*r-0.141096*r2+0.142539*r3;
    let scale=0.818887+0.156573*r-0.669738*r2+0.262344*r3;
    let fdr=vec3(1.0)-(1.0-r2)*base-material.reflectance*scale;
    return (1.0/3.14159265358979323846)*material.diffuse*fdr;
}
fn evaluate_diffuse_response(albedo:vec3<f32>,roughness:f32,metallic:f32,
    specular_tint:vec3<f32>,use_metallic_specular:bool,
    dielectric:vec3<f32>,metal_diffuse:vec3<f32>)->vec3<f32> {
    return evaluate_mapped_diffuse_response(material_lighting_values(albedo,roughness,metallic,
        specular_tint,use_metallic_specular,dielectric,metal_diffuse),roughness);
}
fn material_reflectance(albedo:vec3<f32>,roughness:f32,metallic:f32,specular_tint:vec3<f32>,
    use_metallic_specular:bool,dielectric:vec3<f32>)->vec3<f32> {
    return material_lighting_values(albedo,roughness,metallic,specular_tint,
        use_metallic_specular,dielectric,vec3(0.0)).reflectance;
}

fn evaluate_sh_irradiance(direction: vec3<f32>, probe_index: u32) -> vec3<f32> {
    let x = direction.x;
    let y = direction.y;
    let z = direction.z;
    let c1 = 0.4290427654048917;
    let c2 = 0.5116633539732442;
    let c3 = 0.2477079561003757;
    let c4 = 0.886226925452758;

    return max(
        vec3<f32>(0.0),
        c1 * (x * x - y * y) * probe_sh[probe_index][8].rgb
            + c3 * (3.0 * z * z - 1.0) * probe_sh[probe_index][6].rgb
            + c4 * probe_sh[probe_index][0].rgb
            + 2.0 * c1 * (probe_sh[probe_index][4].rgb * x * y + probe_sh[probe_index][7].rgb * x * z + probe_sh[probe_index][5].rgb * y * z)
            + 2.0 * c2 * (probe_sh[probe_index][3].rgb * x + probe_sh[probe_index][1].rgb * y + probe_sh[probe_index][2].rgb * z),
    );
}

// Match the game's three principal SH sample directions and 0.5 scale for a
// front-facing, unoccluded surface. Bevy does not supply the game's bent-normal
// AO term, so the authored coefficients remain unoccluded here.
fn probe_diffuse(normal: vec3<f32>, probe_index: u32) -> vec3<f32> {
    // Probe coefficients are authored in Dishonored 2 world XYZ. The viewer's
    // camera basis maps game XYZ to view XZY with a sign flip on game Y.
    let n = normalize(vec3<f32>(normal.x, -normal.z, normal.y));
    let helper = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.z) > 0.999);
    let tangent = normalize(cross(helper, n));
    let bitangent = cross(n, tangent);
    let r0 = vec3<f32>(0.7071, 0.0, 0.7071);
    let r1 = vec3<f32>(-0.35355, -0.61237, 0.7071);
    let r2 = vec3<f32>(-0.35355, 0.61237, 0.7071);
    let d0 = r0.x * tangent + r0.y * bitangent + r0.z * n;
    let d1 = r1.x * tangent + r1.y * bitangent + r1.z * n;
    let d2 = r2.x * tangent + r2.y * bitangent + r2.z * n;
    return 0.3535533906 * (
        evaluate_sh_irradiance(d0, probe_index)
        + evaluate_sh_irradiance(d1, probe_index)
        + evaluate_sh_irradiance(d2, probe_index)
    );
}
