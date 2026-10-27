//! 1f2e30/1f2c50: publish live stage slots in their retained index order.
use super::{ParticleInstance, SpriteView};
use crate::{
    float::{add, multiply, subtract},
    gameplay::{particles::simulation::StageSimulation, reference_frame::ReferenceFrame},
};

/// The view owner prepares the admission plane (1ee5a0), including the
/// descriptor's minimum distance. The stage transform is identity for a
/// world-space stage and the system pose for a local-space stage.
pub fn stage_instances(
    stage: &StageSimulation,
    sprite_index: usize,
    stage_to_world: ReferenceFrame,
    view: &SpriteView,
    query_visibility: f32,
    alpha_scale: f32,
    admission_plane: [f32; 4],
    minimum_distance: f32,
    output: &mut Vec<ParticleInstance>,
) -> Result<Option<[[f32; 3]; 2]>, String> {
    let definition = &stage.declaration.stages[stage.index];
    let descriptor = definition
        .sprites
        .get(sprite_index)
        .ok_or("particle publication sprite index absent")?;
    if !descriptor.enabled {
        return Ok(None);
    }
    let states = stage
        .sprites
        .get(sprite_index)
        .ok_or("particle publication sprite slots absent")?;
    let view = view.with_query_visibility(descriptor, query_visibility, alpha_scale)?;
    let view = SpriteView {
        camera_position: view.camera_position,
        camera_axes: view.camera_axes,
        custom_basis: view.custom_basis,
        emitter_basis: stage_to_world.basis,
        owner_colour: view.owner_colour,
        owner_scale: view.owner_scale,
    };
    // 1edaf0 initializes +/-1e30; 1edb20 accumulates published centres.
    let mut bounds = [
        [f32::from_bits(0x7149_f2ca); 3],
        [f32::from_bits(0xf149_f2ca); 3],
    ];
    let first = output.len();
    for (slot, particle) in stage.particles.iter() {
        let mut state = particle.state;
        let transform = |value: [f32; 3]| {
            std::array::from_fn(|i| {
                // 1f2c50 gives X's accumulator a different initial order.
                let (first, second) = if i == 0 { (1, 0) } else { (0, 1) };
                add(
                    add(
                        multiply(value[first], stage_to_world.basis[first][i]),
                        multiply(value[second], stage_to_world.basis[second][i]),
                    ),
                    multiply(value[2], stage_to_world.basis[2][i]),
                )
            })
        };
        let position = transform(state.position);
        state.position = std::array::from_fn(|i| add(position[i], stage_to_world.position[i]));
        let distance = add(
            add(
                add(
                    multiply(admission_plane[1], state.position[1]),
                    multiply(state.position[0], admission_plane[0]),
                ),
                multiply(admission_plane[2], state.position[2]),
            ),
            admission_plane[3],
        );
        if !(minimum_distance <= distance) {
            continue;
        }
        state.velocity = transform(state.velocity);
        let initial = states[slot]
            .as_ref()
            .ok_or("live particle has no initialized sprite slot")?;
        let instance = ParticleInstance::new(descriptor, initial, &state, particle.seed, &view)?;
        for i in 0..3 {
            let position = instance.position[i];
            if !(bounds[0][i] < position) {
                bounds[0][i] = position;
            }
            if !(bounds[1][i] > position) {
                bounds[1][i] = position;
            }
        }
        output.push(instance);
    }
    if first == output.len() {
        return Ok(None);
    }
    // 1ee520 expands by the derived descriptor radius and query-adjusted scale.
    let radius = multiply(descriptor.bounding_radius()?, view.owner_scale);
    for i in 0..3 {
        bounds[0][i] = subtract(bounds[0][i], radius);
        bounds[1][i] = add(radius, bounds[1][i]);
    }
    Ok(Some(bounds))
}
