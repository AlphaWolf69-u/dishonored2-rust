struct WaterNormalQuery {
    position:vec4<f32>, // world XY, camera distance, bitangent sign
    normal:vec4<f32>, // world normal, horizontal flag
    tangent:vec4<f32>,
}
@group(0) @binding(0) var<uniform> parameters:WaterNormalParameters;
@group(0) @binding(1) var permutation:texture_2d<f32>;
@group(0) @binding(2) var gradients:texture_2d<f32>;
@group(0) @binding(3) var<storage,read> queries:array<WaterNormalQuery>;
@group(0) @binding(4) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(5) var point_wrap:sampler;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];
    let height=water_wave_height(parameters,q.position.xy,q.position.z,permutation,gradients,point_wrap);
    let detail=water_wave_normal(parameters,q.position.xy,q.position.z,permutation,gradients,point_wrap);
    let normal=water_surface_normal(parameters,q.position.xy,q.position.z,q.normal.w!=0.0,
        q.normal.xyz,q.tangent.xyz,q.position.w,permutation,gradients,point_wrap);
    output[id.x*2u]=vec4(height,detail);
    output[id.x*2u+1u]=vec4(normal,0.0);
}
