// Shared installed world-shadow VS clip arithmetic: MUL Y, MAD X,
// MAD Z, ADD translation. Inputs are original game-space world position
// and the original VP rows; this is not a rigid-model MVP/depth hack.
fn shadow_world_project(world: vec4<f32>, x: vec4<f32>, y: vec4<f32>,
    z: vec4<f32>, w: vec4<f32>) -> vec4<f32>
{
    let column_x = vec4<f32>(x.x, y.x, z.x, w.x);
    let column_y = vec4<f32>(x.y, y.y, z.y, w.y);
    let column_z = vec4<f32>(x.z, y.z, z.z, w.z);
    let column_w = vec4<f32>(x.w, y.w, z.w, w.w);
    return fma(column_z, vec4<f32>(world.z),
        fma(column_x, vec4<f32>(world.x), column_y * world.y)) + column_w;
}
