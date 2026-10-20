//! Original finite reverse-depth projection transported through Bevy's camera.
//! FUN_140890b50/FUN_140890ac0 and render_perspective_rows define
//! the coefficients. An infinite host projection changes depth reduction even
//! when the displayed scene appears the same. Full-game capture 2026-10-17:
//! near=.01, far=7000, P22=1.42857334e-6, P23=.0100000137.

use bevy::{
    camera::{CameraProjection, PerspectiveProjection, SubCameraView},
    math::{Mat4, Vec3A},
};

#[derive(Clone, Debug)]
pub struct GameProjection {
    pub reference_horizontal_degrees: f32,
    pub near: f32,
    pub far: f32,
    width: u32,
    height: u32,
}

impl GameProjection {
    pub fn new(reference_horizontal_degrees: f32, near: f32, far: f32) -> Self {
        Self {
            reference_horizontal_degrees,
            near,
            far,
            width: 16,
            height: 9,
        }
    }

    pub fn rows(&self) -> [[f32; 4]; 4] {
        let (horizontal, vertical) = crate::tome::render_fov_radians(
            self.reference_horizontal_degrees,
            self.width,
            self.height,
            1.0,
        )
        .expect("valid native gameplay camera field of view and extent");
        let right = self.near * (horizontal * 0.5).tan();
        let top = self.near * (vertical * 0.5).tan();
        crate::tome::render_perspective_rows(
            [-right, right],
            [-top, top],
            self.near,
            self.far,
        )
        .expect("valid native gameplay camera projection bounds")
    }

    fn host_perspective(&self) -> PerspectiveProjection {
        PerspectiveProjection {
            fov: crate::tome::render_vertical_fov_radians(
                self.reference_horizontal_degrees,
            )
            .expect("valid native gameplay camera field of view"),
            aspect_ratio: self.width as f32 / self.height as f32,
            near: self.near,
            far: self.far,
            ..Default::default()
        }
    }
}

impl CameraProjection for GameProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.rows()).transpose()
    }

    fn get_clip_from_view_for_sub(&self, sub_view: &SubCameraView) -> Mat4 {
        // Bevy sub-view transport crops XY; preserve the original depth range.
        // This does not select the game's separate custom/subrect view modes.
        let mut matrix = self.host_perspective().get_clip_from_view_for_sub(sub_view);
        let rows = self.rows();
        matrix.z_axis.z = rows[2][2];
        matrix.w_axis.z = rows[2][3];
        matrix
    }

    fn update(&mut self, width: f32, height: f32) {
        assert!(width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0);
        self.width = width as u32;
        self.height = height as u32;
    }

    fn far(&self) -> f32 {
        self.far
    }

    fn get_frustum_corners(&self, near: f32, far: f32) -> [Vec3A; 8] {
        self.host_perspective().get_frustum_corners(near, far)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_projection_matches_full_game_depth_capture() {
        let mut projection = GameProjection::new(110.0, 0.01, 7000.0);
        projection.update(1920.0, 1080.0);
        let rows = projection.rows();
        assert_eq!(rows[0][0].to_bits(), 0.7002075910568237f32.to_bits());
        assert_eq!(rows[1][1].to_bits(), 1.2448134422302246f32.to_bits());
        assert_eq!(rows[2][2].to_bits(), 1.4285733413998969e-6f32.to_bits());
        assert_eq!(rows[2][3].to_bits(), 0.010000013746321201f32.to_bits());
        let matrix = projection.get_clip_from_view();
        assert_eq!(matrix.row(2).to_array(), rows[2]);
        assert!(
            matrix
                .project_point3(bevy::math::Vec3::new(0.0, 0.0, -7000.0))
                .z
                .abs()
                < 1e-12
        );
    }
}
