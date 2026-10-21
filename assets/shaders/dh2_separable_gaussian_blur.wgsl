// PostFX_Separable_Gaussian_Blur. Compose after dh2_point_sampling.wgsl.
struct Parameters {pixel_scale:vec4<f32>,direction_boost:vec4<f32>,}
@group(0) @binding(0) var<uniform> p:Parameters;
@group(0) @binding(1) var source:texture_2d<f32>;
struct Vertex { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32>, }
@vertex fn vertex(@builtin(vertex_index) index:u32)->Vertex {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return Vertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
fn gaussian_tap(uv:vec2<f32>,direction:vec2<f32>,scale:vec2<f32>,offset:f32)->f32 {
    return point_mirror_sample(source,fma(-direction,vec2(offset),uv)*scale).r;
}
@fragment fn fragment(in:Vertex)->@location(0) f32 {
    let scale=p.pixel_scale.zw;
    // Installed PC stage4213 squares scale first and uses MAD for coordinates.
    // Separate multiply/subtract can choose another point-sampled texel at a tie.
    let pixel_size=(scale*scale)*p.pixel_scale.xy;
    let direction=p.direction_boost.xy*pixel_size;
    // Source [unroll] and installed4213's actual accumulation order.
    var sum=gaussian_tap(in.uv,direction,scale,-3.0)*0.09;
    sum=fma(gaussian_tap(in.uv,direction,scale,-4.0),0.05,sum);
    sum=fma(gaussian_tap(in.uv,direction,scale,-2.0),0.12,sum);
    sum=fma(gaussian_tap(in.uv,direction,scale,-1.0),0.15,sum);
    sum=fma(gaussian_tap(in.uv,direction,scale, 0.0),0.16,sum);
    sum=fma(gaussian_tap(in.uv,direction,scale, 1.0),0.15,sum);
    sum=fma(gaussian_tap(in.uv,direction,scale, 2.0),0.12,sum);
    sum=fma(gaussian_tap(in.uv,direction,scale, 3.0),0.09,sum);
    sum=fma(gaussian_tap(in.uv,direction,scale, 4.0),0.05,sum);
    if p.direction_boost.z>0.0 {return sum*((1.0/0.98)*p.direction_boost.z);}
    return clamp(sum*(1.0/0.98),0.0,1.0);
}
