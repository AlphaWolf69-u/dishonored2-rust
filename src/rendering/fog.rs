//! Authored atmosphere calculations, separate from forward surface lighting.

pub mod world;

/// Signed linear-depth contribution to the separate interior fog-volume target.
/// arkInteriorVolume writes ONE/ONE, with depth testing/writes disabled. This
/// value is an input to distance fog, never an opaque forward colour surface.
pub fn interior_volume_depth(scene_depth: f32, volume_depth: f32, front_face: bool) -> f32 {
    if front_face {
        -volume_depth.clamp(0., scene_depth)
    } else {
        scene_depth.min(volume_depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interior_boundaries_accumulate_only_the_depth_inside_the_scene() {
        assert_eq!(
            interior_volume_depth(100., 20., true) + interior_volume_depth(100., 60., false),
            40.
        );
        assert_eq!(
            interior_volume_depth(40., 20., true) + interior_volume_depth(40., 60., false),
            20.
        );
        assert_eq!(
            interior_volume_depth(10., 20., true) + interior_volume_depth(10., 60., false),
            0.
        );
        assert_eq!(
            interior_volume_depth(100., -20., true) + interior_volume_depth(100., 60., false),
            60.
        );
    }
}
