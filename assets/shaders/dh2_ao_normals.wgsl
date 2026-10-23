@group(1) @binding(0) var ao_linear:texture_2d<f32>;
@group(1) @binding(1) var ao_point:sampler;
fn ao_fetch_position(uv:vec2<f32>)->vec3<f32> {
    return ao_view_position(uv,textureSampleLevel(ao_linear,ao_point,uv,0.0).r);
}
fn ao_min_difference(p:vec3<f32>,positive:vec3<f32>,negative:vec3<f32>)->vec3<f32> {
    let a=positive-p;let b=p-negative;return select(b,a,dot(a,a)<dot(b,b));
}
@fragment fn ao_reconstruct_normal(input:AoVertex)->@location(0) vec4<f32> {
    let uv=input.uv;let p=ao_fetch_position(uv);
    let dx=vec2(ao.p[1].z,0.0);let dy=vec2(0.0,ao.p[1].w);
    let x=ao_min_difference(p,ao_fetch_position(uv+dx),ao_fetch_position(uv-dx));
    let y=ao_min_difference(p,ao_fetch_position(uv+dy),ao_fetch_position(uv-dy));
    return vec4(normalize(cross(x,y))*0.5+vec3(0.5),0.0);
}
