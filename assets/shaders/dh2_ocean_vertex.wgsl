// arkOcean vertex preparation. Compose after the shared ocean surface shader.
// Matrices, patch coordinates and camera use game space throughout.
struct OceanVertexParameters {
    clip_from_world:mat4x4<f32>,
    previous_clip_from_world:mat4x4<f32>,
    view_row:vec4<f32>,
    patch_transform:vec4<f32>,
    // PerlinBlendEnd, jitterRelativeToPreviousFrame.xy, unused.
    perlin_end_jitter:vec4<f32>,
}
struct OceanMeshVertex {
    clip:vec4<f32>,
    previous_clip:vec4<f32>,
    world:vec3<f32>,
    legacy:vec4<f32>,
}
fn ocean_mesh_vertex(surface:OceanSurfaceParameters,vertex:OceanVertexParameters,
    grid:vec2<f32>,depth_pass:bool,
    permutation:texture_2d<f32>,gradients:texture_2d<f32>,
    point_wrap:sampler,bilinear_wrap:sampler,bilinear_clamp:sampler,
    shore_distance:texture_2d<f32>,current_displacement:texture_2d<f32>,
    previous_displacement:texture_2d<f32>,collision_height:texture_2d<f32>,
    fft_foam:texture_2d<f32>,amplitude_mask:texture_2d<f32>)->OceanMeshVertex {
    let undisplaced=vec3(grid*vertex.patch_transform.w,0.0)+vertex.patch_transform.xyz;
    let camera_distance=distance(surface.camera.xyz,undisplaced);
    let blend=ocean_smooth_step(surface.perlin.x,vertex.perlin_end_jitter.x,camera_distance);
    var uv=undisplaced.xy/surface.spatial.w+vec2(0.5);
    uv.x=1.0-uv.x;
    let amplitude=textureSampleLevel(amplitude_mask,bilinear_clamp,uv,0.0).r;
    let input=OceanSurfaceInput(undisplaced.xy,camera_distance,blend,amplitude,false,0.0);
    let displacement=ocean_surface_displacement(surface,input,permutation,gradients,
        point_wrap,bilinear_wrap,bilinear_clamp,shore_distance,current_displacement,collision_height,fft_foam);
    let world=undisplaced+displacement.displacement;
    var previous=vec4(0.0);
    var legacy=vec4(0.0);
    if depth_pass {
        // Source history uses current shore/Perlin time and the previous FFT image.
        let old_displacement=ocean_surface_displacement(surface,input,permutation,gradients,
            point_wrap,bilinear_wrap,bilinear_clamp,shore_distance,previous_displacement,collision_height,fft_foam);
        previous=vertex.previous_clip_from_world*vec4(undisplaced+old_displacement.displacement,1.0);
    } else {
        legacy=vec4(undisplaced.xy,blend,-dot(vec4(world,1.0),vertex.view_row));
    }
    return OceanMeshVertex(vertex.clip_from_world*vec4(world,1.0),previous,world,legacy);
}
