//! Render-view inputs published by 1ff460 before sprite generation.
use crate::float::{add, multiply, subtract};

#[derive(Clone, Copy)]
pub struct ParticleCamera {
    pub position: [f32; 3],
    pub axes: [[f32; 3]; 3],
    pub admission_plane: [f32; 4],
    pub emitter_distance: f32,
}

impl ParticleCamera {
    /// The view supplies both origins. Keep the primary on ties or unordered
    /// comparisons; this is not the separate bounds-centre LOD distance.
    pub fn select(
        emitter: [f32; 3],
        primary: [f32; 3],
        alternate: [f32; 3],
        axes: [[f32; 3]; 3],
    ) -> Self {
        let squared_distance = |origin: [f32; 3]| {
            let delta: [f32; 3] = std::array::from_fn(|i| subtract(emitter[i], origin[i]));
            add(
                add(multiply(delta[1], delta[1]), multiply(delta[0], delta[0])),
                multiply(delta[2], delta[2]),
            )
        };
        let primary_distance = squared_distance(primary);
        let alternate_distance = squared_distance(alternate);
        let (position, distance) = if alternate_distance < primary_distance {
            (alternate, alternate_distance)
        } else {
            (primary, primary_distance)
        };
        // 1faa50 uses the first camera axis and flips the dot product's sign
        // bit. 1411ad278 uses SQRTSS for finite nonnegative distances.
        let normal = axes[0];
        let dot = add(
            add(
                multiply(normal[1], position[1]),
                multiply(normal[0], position[0]),
            ),
            multiply(normal[2], position[2]),
        );
        Self {
            position,
            axes,
            admission_plane: [
                normal[0],
                normal[1],
                normal[2],
                f32::from_bits(dot.to_bits() ^ 0x8000_0000),
            ],
            emitter_distance: distance.sqrt(),
        }
    }
}
