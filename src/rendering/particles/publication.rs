//! 1f2e30/1f2c50: publish live stage slots in their retained index order.
use super::{ParticleInstance, SpriteView};
use crate::{
    float::{add, multiply, subtract},
    gameplay::{particles::simulation::StageSimulation, reference_frame::ReferenceFrame},
};

/// What the sprite and mesh publishers read from a stage. Havok stages
/// (1401e5440 / 1401e4f50) draw each body with its own basis: the
/// AngularMotion orientation, with the position moved back by the collision
/// hull centroid offset in that basis.
pub struct DrawStage<'a> {
    pub definition: &'a crate::assets::fx::particle_stage::ParticleStageAsset,
    pub particles: &'a crate::gameplay::particles::stage::ParticleStage,
    pub sprites: &'a [Vec<Option<crate::gameplay::particles::sprite::SpriteInitialState>>],
    pub meshes: &'a [Vec<Option<crate::gameplay::particles::mesh_appearance::MeshInitialState>>],
    pub bodies: Option<(&'a dyn Fn(usize) -> Option<[[f32; 3]; 3]>, [f32; 3])>,
}
impl<'a> DrawStage<'a> {
    pub fn ordinary(stage: &'a StageSimulation) -> Self {
        Self {
            definition: &stage.declaration.stages[stage.index],
            particles: &stage.particles,
            sprites: &stage.sprites,
            meshes: &stage.meshes,
            bodies: None,
        }
    }

    /// The body basis and centroid-shifted position (identity basis without
    /// AngularMotion).
    pub fn body(&self, slot: usize, position: [f32; 3]) -> Option<([[f32; 3]; 3], [f32; 3])> {
        let (basis, offset) = self.bodies?;
        let Some(r) = basis(slot) else {
            return Some(([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], position));
        };
        let o = offset;
        Some((
            r,
            [
                subtract(position[0], add(add(multiply(o[1], r[1][0]), multiply(o[0], r[0][0])), multiply(o[2], r[2][0]))),
                subtract(position[1], add(add(multiply(o[0], r[0][1]), multiply(o[1], r[1][1])), multiply(o[2], r[2][1]))),
                subtract(position[2], add(add(multiply(o[0], r[0][2]), multiply(o[1], r[1][2])), multiply(o[2], r[2][2]))),
            ],
        ))
    }
}

/// The view owner prepares the admission plane (1ee5a0), including the
/// descriptor's minimum distance. The stage transform is identity for a
/// world-space stage and the system pose for a local-space stage.
pub fn stage_instances(
    stage: &DrawStage,
    sprite_index: usize,
    stage_to_world: ReferenceFrame,
    view: &SpriteView,
    query_visibility: f32,
    alpha_scale: f32,
    admission_plane: [f32; 4],
    minimum_distance: f32,
    order: Option<&[usize]>,
    output: &mut Vec<ParticleInstance>,
) -> Result<Option<[[f32; 3]; 2]>, String> {
    let definition = stage.definition;
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
    for (slot, particle) in stage.particles.in_order(order) {
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
        let mut body_view = view;
        if let Some((basis, position)) = stage.body(slot, state.position) {
            body_view.emitter_basis = basis;
            state.position = position;
        }
        let instance = ParticleInstance::new(descriptor, initial, &state, particle.seed, &body_view)?;
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
