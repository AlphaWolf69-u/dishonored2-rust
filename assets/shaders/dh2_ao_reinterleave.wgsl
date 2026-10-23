@group(1) @binding(0) var ao_atlas:texture_2d<f32>;
@fragment fn ao_reinterleave(input:AoVertex)->@location(0) vec4<f32> {
    let pixel=vec2<i32>(input.position.xy);let size=vec2<i32>(textureDimensions(ao_atlas))/4;
    let tile=pixel&vec2(3);let p=(pixel>>vec2(2u))+tile*size;
    let value=clamp(textureLoad(ao_atlas,p,0).r,0.0,1.0);
    return vec4(pow(value,ao.p[4].z));
}
