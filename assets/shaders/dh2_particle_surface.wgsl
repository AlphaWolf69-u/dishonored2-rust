#define_import_path dh2_particle_surface

// A particle renderer drawn as a material surface with direct instancing
// (1401fbad0 -> 1402bb550 -> 1402bace0). 1eaa40 writes one model matrix per
// instance (its "previous" rows are identical) and the packed particle
// parameters; the indirect-lighting record is the system's shared one.
struct ParticleSurfaceInstance {
    // Row-major 3x4 world-from-local, already in the backend's view axes.
    model: array<vec4<f32>, 3>,
    // colour (u8x4), flipbook_seed_uv, size (half), lighting record index.
    parameters: vec4<u32>,
}

@group(4) @binding(0) var<storage, read> particle_surface_instances: array<ParticleSurfaceInstance>;

// DrawMesh's direct-instancing branch: this entity's mesh uniform and the
// first record of its instances ($instanceBufferOffset).
struct ParticleSurfaceDraw {
    mesh_index: u32,
    first: u32,
}
var<immediate> particle_surface_draw: ParticleSurfaceDraw;

fn particle_surface_instance(instance_index: u32) -> ParticleSurfaceInstance {
    return particle_surface_instances[particle_surface_draw.first + instance_index];
}

fn particle_surface_world_from_local(instance: ParticleSurfaceInstance) -> mat4x4<f32> {
    return transpose(mat4x4<f32>(instance.model[0], instance.model[1], instance.model[2], vec4(0.0, 0.0, 0.0, 1.0)));
}

// particles_instancing GetParticleColor: colour * unpack4_u8tof32(color).
fn particle_surface_colour(instance: ParticleSurfaceInstance, colour: vec4<f32>) -> vec4<f32> {
    return colour * unpack4x8unorm(instance.parameters.x);
}
