struct OceanColourView {
    surface:OceanSurfaceParameters,
    vertex:OceanVertexParameters,
    reflection:WaterReflectionParameters,
    refraction:WaterRefractionParameters,
    shadow:WaterShadowParameters,
    water:WaterSurfaceParameters,
    colour:OceanColourParameters,
    output:ColourOutputParameters,
    sky_radius:vec4<f32>,
    // The ocean material's UseSkyCubeMapOption; remaining lanes unused.
    options:vec4<u32>,
}
@group(0) @binding(0) var<uniform> ocean:OceanColourView;
@group(1) @binding(0) var permutation:texture_2d<f32>;
@group(1) @binding(1) var gradients:texture_2d<f32>;
@group(1) @binding(2) var shore:texture_2d<f32>;
@group(1) @binding(3) var displacement:texture_2d<f32>;
@group(1) @binding(4) var previous:texture_2d<f32>;
@group(1) @binding(5) var collision:texture_2d<f32>;
@group(1) @binding(6) var fft_foam:texture_2d<f32>;
@group(1) @binding(7) var amplitude:texture_2d<f32>;
@group(1) @binding(8) var point_wrap:sampler;
@group(1) @binding(9) var bilinear_wrap:sampler;
@group(1) @binding(10) var bilinear_clamp:sampler;
@group(2) @binding(0) var projected_height:texture_2d<f32>;
@group(2) @binding(1) var foam_texture:texture_2d<f32>;
@group(2) @binding(2) var foam_depth:texture_2d<f32>;
@group(2) @binding(3) var opaque_depth:texture_depth_2d;
@group(2) @binding(4) var reduced_depth:texture_2d<f32>;
@group(2) @binding(5) var hdr:texture_2d<f32>;
@group(2) @binding(6) var fog:texture_2d<f32>;
@group(2) @binding(7) var caustics:texture_2d_array<f32>;
@group(2) @binding(8) var shadow:texture_depth_2d_array;
@group(2) @binding(9) var sky:texture_cube<f32>;
@group(2) @binding(10) var probes:texture_cube_array<f32>;
@group(2) @binding(11) var comparison:sampler_comparison;
struct OceanColourVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(perspective,center) world:vec3<f32>,
    @location(1) @interpolate(perspective,center) legacy:vec4<f32>,
}
@vertex fn ocean_colour_vertex(@location(0) grid:vec2<f32>)->OceanColourVertex {
    let vertex=ocean_mesh_vertex(ocean.surface,ocean.vertex,grid,false,
        permutation,gradients,point_wrap,bilinear_wrap,bilinear_clamp,
        shore,displacement,previous,collision,fft_foam,amplitude);
    return OceanColourVertex(vertex.clip,vertex.world,vertex.legacy);
}
fn ocean_colour(vertex:OceanColourVertex,front:bool,parameters:OceanColourView)->vec4<f32> {
    let ocean=parameters;
    let foam_uv=vertex.legacy.xy*0.04+vec2(sin(ocean.surface.clock_view.x)*0.01);
    let foam=textureSample(foam_texture,bilinear_wrap,foam_uv).r;
    // global::screenPosToTexcoord deliberately ignores the bias.xy lanes.
    let uv=vertex.position.xy*ocean.colour.pixel_to_texture.zw;
    let depth=textureSample(foam_depth,bilinear_wrap,uv).r;
    let input=ocean_colour_input(ocean.colour,ocean.surface,ocean.reflection.view_from_world,
        vertex.world,vertex.legacy,vertex.position.xy,front,ocean.refraction.unproject.w,foam,depth,
        permutation,gradients,point_wrap,bilinear_wrap,bilinear_clamp,
        shore,displacement,collision,fft_foam,amplitude,projected_height);
    let environment=WaterEnvironment(ocean.sky_radius.xyz,ocean.sky_radius.w,false,false,ocean.options.x!=0u);
    let result=water_surface(ocean.water,ocean.reflection,ocean.refraction,ocean.shadow,environment,input,
        opaque_depth,reduced_depth,hdr,fog,caustics,probes,sky,shadow,
        bilinear_clamp,bilinear_wrap,bilinear_wrap,bilinear_wrap,comparison);
    return vec4(colour_output(ocean.output,result.rgb),result.a);
}
@fragment fn ocean_colour_fragment(vertex:OceanColourVertex,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    return ocean_colour(vertex,front,ocean);
}
