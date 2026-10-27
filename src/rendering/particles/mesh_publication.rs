//! 1eaa40 publishes mesh matrices and instance parameters from retained slots.
use super::{PackedParticleParameters, ParticleInstance, SpriteView};
use crate::assets::fx::particle_mesh::ParticleMeshAsset;
use crate::{
    float::{add, divide, multiply, subtract},
    gameplay::{
        particles::{
            mesh_appearance::MeshInitialState, orientation::normalize, simulation::StageSimulation,
            ParticleState,
        },
        reference_frame::{basis_product, ReferenceFrame},
    },
};

fn instance(
    descriptor: &ParticleMeshAsset,
    initial: &MeshInitialState,
    particle: &ParticleState,
    particle_seed: u32,
    view: &SpriteView,
) -> Result<ParticleInstance, String> {
    let appearance = &descriptor.appearance;
    let age = particle.age;
    let mut seed = particle_seed;
    let mut velocity = particle.velocity;
    let squared = add(
        add(
            multiply(velocity[1], velocity[1]),
            multiply(velocity[0], velocity[0]),
        ),
        multiply(velocity[2], velocity[2]),
    );
    let reciprocal = divide(1., squared.max(f32::MIN_POSITIVE).sqrt());
    velocity = velocity.map(|v| multiply(v, reciprocal));
    let speed = multiply(reciprocal, squared);
    let offset = appearance
        .velocity_offset_over_life
        .sample(age, &mut seed)?;
    let mut position =
        std::array::from_fn(|i| add(particle.position[i], multiply(velocity[i], offset)));
    let life = appearance.size_over_life.sample(age, &mut seed)?;
    let over_velocity = appearance.size_over_velocity.sample(speed, &mut seed)?;
    let size: [f32; 3] = std::array::from_fn(|i| {
        multiply(
            multiply(multiply(initial.size[i], life[i]), view.owner_scale),
            over_velocity[i],
        )
    });
    let mut to_camera = std::array::from_fn(|i| subtract(view.camera_position[i], position[i]));
    normalize(&mut to_camera);
    let axes = descriptor.orientation.sample(
        age,
        &mut seed,
        view.custom_basis,
        view.emitter_basis,
        to_camera,
        velocity,
        view.camera_axes,
    )?;
    let rotation = appearance.rotation_over_life.sample(age, &mut seed)?;
    let angles = std::array::from_fn(|i| add(rotation[i], initial.rotation[i]));
    let rotation = crate::gameplay::particles::spawn::rotation_basis(
        angles,
        descriptor.rotation_order,
        ReferenceFrame::default().basis,
    )?;
    let mut axes = basis_product(rotation, axes);
    for i in 0..3 {
        axes[i] = axes[i].map(|v| multiply(v, size[i]));
    }
    let pivot = appearance.pivot_offset_over_life.sample(age, &mut seed)?;
    let pivot: [f32; 3] = std::array::from_fn(|i| add(pivot[i], initial.pivot[i]));
    position = std::array::from_fn(|i| {
        add(
            add(
                add(position[i], multiply(axes[1][i], pivot[1])),
                multiply(axes[0][i], pivot[0]),
            ),
            multiply(axes[2][i], pivot[2]),
        )
    });
    let colour = initial.colour_at(appearance, age, view.owner_colour, &mut seed)?;
    let radius = add(
        add(multiply(size[0], size[0]), multiply(size[1], size[1])),
        multiply(size[2], size[2]),
    )
    .sqrt();
    Ok(ParticleInstance {
        position,
        axes,
        parameters: PackedParticleParameters::new(
            colour,
            particle_seed,
            initial.flip_bits,
            initial.animation_frame.unwrap_or(age),
            radius,
        ),
    })
}

pub fn stage_instances(
    stage: &StageSimulation,
    index: usize,
    transform: ReferenceFrame,
    view: &SpriteView,
    query: f32,
    alpha_scale: f32,
    output: &mut Vec<ParticleInstance>,
) -> Result<(), String> {
    let descriptor = &stage.declaration.stages[stage.index].meshes[index];
    let mut view = *view;
    view.emitter_basis = transform.basis;
    view.owner_colour[3] = multiply(
        multiply(
            descriptor.query_alpha.sample_visibility(query)?,
            view.owner_colour[3],
        ),
        alpha_scale,
    );
    view.owner_scale = multiply(
        descriptor.query_size.sample_visibility(query)?,
        view.owner_scale,
    );
    for (slot, particle) in stage.particles.iter() {
        let mut state = particle.state;
        let rotate = |v: [f32; 3]| {
            std::array::from_fn(|i| {
                let (a, b) = if i == 0 { (1, 0) } else { (0, 1) };
                add(
                    add(
                        multiply(v[a], transform.basis[a][i]),
                        multiply(v[b], transform.basis[b][i]),
                    ),
                    multiply(v[2], transform.basis[2][i]),
                )
            })
        };
        let position = rotate(state.position);
        state.position = std::array::from_fn(|i| add(position[i], transform.position[i]));
        state.velocity = rotate(state.velocity);
        let initial = stage.meshes[index][slot]
            .as_ref()
            .ok_or("live particle has no mesh slot")?;
        output.push(instance(descriptor, initial, &state, particle.seed, &view)?);
    }
    Ok(())
}
