@group(1) @binding(0) var ao_quarter:texture_2d_array<f32>;
@group(1) @binding(1) var ao_normals:texture_2d<f32>;
@group(1) @binding(2) var ao_point:sampler;
@group(1) @binding(3) var<uniform> ao_jitters:array<vec4<f32>,16>;
fn ao_sample_position(uv:vec2<f32>,slice:i32)->vec3<f32> {
    return ao_view_position(uv,textureSampleLevel(ao_quarter,ao_point,uv,slice,0.0).r);
}
fn ao_contribution(p:vec3<f32>,n:vec3<f32>,s:vec3<f32>,negative_inverse_radius:f32)->f32 {
    let v=s-p;let distance=dot(v,v);
    return clamp(dot(n,v)*inverseSqrt(distance)-ao.p[3].w,0.0,1.0)*clamp(distance*negative_inverse_radius+1.0,0.0,1.0);
}
@fragment fn ao_coarse(input:AoVertex)->@location(0) f32 {
    // The host packs the sixteen independent R8 slices into a 4x4 atlas.
    // Local coordinates, samples, quantization and per-slice jitter retain the
    // game's algorithm; no geometry shader or sixteen separate draws are needed.
    let size=vec2<i32>(textureDimensions(ao_quarter));let pixel=vec2<i32>(input.position.xy);
    let tile=pixel/size;let slice=tile.y*4+tile.x;let local=pixel%size;
    let full=vec2<f32>(local)*4.0+vec2<f32>(tile)+vec2(0.5);
    let uv=(full*ao.p[1].xy)*0.25;let p=ao_sample_position(uv,slice);
    var radius=ao.p[3].x/p.z;let scale=min(1.0,ao.p[8].y/radius);radius*=scale;
    if radius<1.0 {return 1.0;}
    let inverse_radius=(1.0/(scale*scale))*ao.p[3].z;
    let normal=textureLoad(ao_normals,vec2<i32>(full),0).xyz*2.0-vec3(1.0);
    let jitter=ao_jitters[slice];let step=radius*0.05;
    let directions=array<vec2<f32>,8>(vec2(1.0,0.0),vec2(0.70710677,0.70710677),vec2(0.0,1.0),
        vec2(-0.70710677,0.70710677),vec2(-1.0,0.0),vec2(-0.70710677,-0.70710677),
        vec2(0.0,-1.0),vec2(0.70710677,-0.70710677));
    var near=0.0;var far=0.0;
    for(var i=0u;i<8u;i++) {
        let d=directions[i];let direction=vec2(d.x*jitter.x-d.y*jitter.y,d.x*jitter.y+d.y*jitter.x);
        var ray=jitter.z*step+1.0;
        let first=uv+round(ray*direction)*ao.p[1].xy;
        near+=ao_contribution(p,normal,ao_sample_position(first,slice),inverse_radius);ray+=step;
        for(var k=1u;k<4u;k++) {
            let sample_uv=uv+round(ray*direction)*ao.p[1].xy;
            far+=ao_contribution(p,normal,ao_sample_position(sample_uv,slice),inverse_radius);ray+=step;
        }
    }
    return clamp(1.0-(near*ao.p[4].x+far*ao.p[4].y)*0.0625,0.0,1.0);
}
