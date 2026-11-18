// Dark Vision's sonar (depth) wave: rendering/darkVisionDepthWaveColor in
// the post-process colour and rendering/darkVisionDepthWaveDisto in the
// screen distortion vectors. World positions are rebuilt from the depth
// buffer in game axes; the wave centre is the record's (renderer +0x4254).
struct DarkVisionWave {
    inverse_view_projection:mat4x4<f32>,
    centre_time:vec4<f32>,      // wave centre, localTime
    radii_power:vec4<f32>,      // DepthWaveColorRadius (inner, outer), DepthWavePower, host units
    extent:vec4<f32>,           // target size, 1/full view height, 0
    noise:vec4<f32>,            // DepthWaveNoisePower, DepthWaveColorNoisePower, DepthWaveColorPower, 0
    edge_thickness:vec4<f32>,   // DepthWaveEdgeThickness (x, y)
    tint:vec4<f32>,             // DepthWaveTint
    edge_tint:vec4<f32>,        // DepthWaveEdgeTint
}
@group(0) @binding(0) var<uniform> wave:DarkVisionWave;
@group(0) @binding(1) var wave_depth:texture_depth_2d;
@group(0) @binding(3) var<storage,read> wave_adaptation:array<vec4<f32>,2>;

@vertex fn dark_vision_wave_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let corner=vec2(f32((index<<1u)&2u),f32(index&2u));
    return vec4(corner*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}

struct WavePoint { uv:vec2<f32>, world:vec3<f32>, distance:f32, hash:f32 }

fn wave_point(pixel:vec2<f32>)->WavePoint {
    let uv=pixel/wave.extent.xy;
    // bevy_backend: the depth target is not filterable; the full-resolution
    // texel under the pixel stands in for the original's linear-clamp fetch.
    let size=vec2<f32>(textureDimensions(wave_depth));
    let texel=min(vec2<u32>(uv*size),vec2<u32>(size)-vec2(1u));
    let depth=textureLoad(wave_depth,texel,0);
    let clip=wave.inverse_view_projection*vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),depth,1.0);
    let world=clip.xyz/clip.w;
    // The shader's per-position hash, offset by the fractional local time.
    let hash=fract(sin(dot(world+vec3(fract(wave.centre_time.w)),vec3(12.9898,78.233,37.8956)))*43758.5453);
    return WavePoint(uv,world,length(wave.centre_time.xyz-world),hash);
}

// A band from the centre out to the outer radius that peaks at the inner one.
fn wave_band(distance:f32,inner:f32,outer:f32)->f32 {
    return smoothstep(0.0,inner,distance)*(1.0-smoothstep(inner,outer,distance));
}

@fragment fn dark_vision_wave_colour(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let point=wave_point(pixel.xy);
    let jitter=mix(1.0,point.hash,wave.noise.x);
    let inner=wave.radii_power.x-jitter;
    let outer=wave.radii_power.y+jitter;
    let thickness=wave.edge_thickness.xy;
    let band=wave_band(point.distance,inner,outer);
    let edge=smoothstep(outer-thickness.x,outer-thickness.y,point.distance)
        *(1.0-smoothstep(outer-thickness.y,outer,point.distance));
    let power=max(band,edge)*wave.radii_power.z;
    if power<0.001 {discard;}
    let colour=mix(1.0,point.hash,wave.noise.y)*wave.noise.z*mix(wave.tint.xyz,wave.edge_tint.xyz,edge);
    // Written as colour / EngineLuminanceFactor in native target units,
    // converted to the host target's pre-exposed units.
    return vec4(colour/wave_adaptation[0].x*wave.radii_power.w,power);
}

@fragment fn dark_vision_wave_distortion(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let point=wave_point(pixel.xy);
    let power=wave_band(point.distance,wave.radii_power.x-point.hash,wave.radii_power.y+point.hash)*wave.radii_power.z;
    if power<0.001 {discard;}
    let screen_hash=fract(sin(dot(point.uv+vec2(fract(wave.centre_time.w)),vec2(12.9898,78.233)))*43758.5453);
    return vec4(0.0,power*screen_hash*wave.extent.z*0.2,0.0,0.0);
}
