// MaterialExposure already selects the camera's actual 32-byte adaptation
// record, rather than a CPU readback or another view's exposure index.
@group(3) @binding(16) var<storage,read> water_view_adaptation:array<vec4<f32>,2>;

@fragment fn water_world_colour_fragment_with_exposure(vertex:WorldProbeColourFragment,
    @builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    return water_world_colour(vertex,front,
        water_world_colour_with_exposure(water,water_view_adaptation[0].x));
}
