//! Whole-system receiver bounds used by particle fog selection.
use crate::{
    float::{add, multiply, subtract},
    gameplay::{particles::simulation::StageSimulation, reference_frame::ReferenceFrame},
};

/// 0fbac0 transforms a box without scale. Its X accumulator starts with Y;
/// the Y/Z accumulators start with X. Preserve that distinction.
pub fn transform(local: [[f32; 3]; 2], pose: ReferenceFrame) -> [[f32; 3]; 2] {
    let center: [f32; 3] = std::array::from_fn(|i| multiply(add(local[1][i], local[0][i]), 0.5));
    let extent: [f32; 3] = std::array::from_fn(|i| subtract(local[1][i], center[i]));
    let world_extent: [f32; 3] = std::array::from_fn(|i| {
        add(
            add(
                multiply(extent[1], pose.basis[1][i]).abs(),
                multiply(extent[0], pose.basis[0][i]).abs(),
            ),
            multiply(extent[2], pose.basis[2][i]).abs(),
        )
    });
    let world_center: [f32; 3] = std::array::from_fn(|i| {
        let (a, b) = if i == 0 { (1, 0) } else { (0, 1) };
        add(
            add(
                add(
                    multiply(center[a], pose.basis[a][i]),
                    multiply(center[b], pose.basis[b][i]),
                ),
                multiply(center[2], pose.basis[2][i]),
            ),
            pose.position[i],
        )
    });
    [
        std::array::from_fn(|i| subtract(world_center[i], world_extent[i])),
        std::array::from_fn(|i| add(world_extent[i], world_center[i])),
    ]
}

/// 1c3b80/1c3890's default root box plus 1ffff0/201480's generated bounds.
/// The declaration reader rejects authored visibility overrides; those must
/// supply their own root bounds before they can use this publication path.
pub fn default_system_bounds(
    stages: &[StageSimulation],
    pose: ReferenceFrame,
    scale: f32,
) -> Result<[[f32; 3]; 2], String> {
    let mut result = [
        pose.position.map(|p| add(-1., p)),
        pose.position.map(|p| add(1., p)),
    ];
    for stage in stages {
        if stage.particles.is_empty() {
            continue;
        }
        let mut local = [
            [f32::from_bits(0x7149_f2ca); 3],
            [f32::from_bits(0xf149_f2ca); 3],
        ];
        for (_, particle) in stage.particles.iter() {
            for i in 0..3 {
                let p = particle.state.position[i];
                if p <= local[0][i] {
                    local[0][i] = p;
                }
                if local[1][i] <= p {
                    local[1][i] = p;
                }
            }
        }
        let definition = &stage.declaration.stages[stage.index];
        let mut radius = 0.;
        // 1efbe0 includes every descriptor, independently of draw admission.
        for sprite in &definition.sprites {
            let candidate = sprite.bounding_radius()?;
            if radius <= candidate {
                radius = candidate;
            }
        }
        let radius = multiply(radius, scale);
        let world = transform(
            local,
            if definition.worldspace {
                ReferenceFrame::default()
            } else {
                pose
            },
        );
        for i in 0..3 {
            let minimum = subtract(world[0][i], radius);
            let maximum = add(world[1][i], radius);
            if minimum <= result[0][i] {
                result[0][i] = minimum;
            }
            if result[1][i] <= maximum {
                result[1][i] = maximum;
            }
        }
    }
    Ok(result)
}
