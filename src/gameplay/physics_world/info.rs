//! hknpWorldCinfo defaults (ca4dc0), named from the installed reflection data.
use crate::gameplay::{body_lod::BodyLodConfig, constraint_solver::step::SolverTuning};

/// Stable references supplied by the game physics owner in0932d0. These are
/// rewrite resource handles, never addresses in the original process.
#[derive(Clone, Copy, Debug)]
pub struct WorldHandles {
    pub persistent_stream_allocator: usize,
    pub material_library: usize,
    pub collision_filter: usize,
}

#[derive(Clone, Debug)]
pub struct WorldInfo {
    pub body_buffer_capacity: u32,
    pub user_body_buffer: Option<usize>,
    pub motion_buffer_capacity: u32,
    pub user_motion_buffer: Option<usize>,
    pub constraint_buffer_capacity: u32,
    pub user_constraint_buffer: Option<usize>,
    pub constraint_group_buffer_capacity: u32,
    pub user_constraint_group_buffer: Option<usize>,
    pub persistent_stream_allocator: Option<usize>,
    pub material_library: Option<usize>,
    pub motion_properties_library: Option<usize>,
    pub quality_library: Option<usize>,
    pub simulation_type: u8,
    pub num_splitter_cells: u32,
    pub gravity: [f32; 4],
    pub enable_contact_caching: bool,
    pub merge_events_before_dispatch: bool,
    pub broad_phase_type: u8,
    pub broad_phase_aabb: [[f32; 4]; 2],
    pub broad_phase_config: Option<usize>,
    pub collision_filter: Option<usize>,
    pub shape_tag_codec: Option<usize>,
    pub collision_tolerance: f32,
    pub relative_collision_accuracy: f32,
    pub aabb_margin: f32,
    pub enable_welding_for_default_objects: bool,
    pub enable_welding_for_critical_objects: bool,
    pub lod: BodyLodConfig,
    pub enable_sdf_edge_collisions: bool,
    pub enable_collide_work_stealing: bool,
    pub solver: SolverTuning,
    pub solver_enable_post_step: bool,
    pub solver_microsteps: u32,
    pub max_approach_speed_for_high_quality_solver: f32,
    pub enable_deactivation: bool,
    pub enable_penetration_recovery: bool,
}

impl Default for WorldInfo {
    fn default() -> Self {
        let mut solver = SolverTuning {
            tau: 0.,
            damping: 0.,
            iterations: 0,
        };
        solver.set_quality(5);
        let mut info = Self {
            body_buffer_capacity: 0x1000,
            user_body_buffer: None,
            motion_buffer_capacity: 0x1000,
            user_motion_buffer: None,
            constraint_buffer_capacity: 0x200,
            user_constraint_buffer: None,
            constraint_group_buffer_capacity: 0x10,
            user_constraint_group_buffer: None,
            persistent_stream_allocator: None,
            material_library: None,
            motion_properties_library: None,
            quality_library: None,
            simulation_type: 1,
            num_splitter_cells: 16,
            gravity: [0., f32::from_bits(0xc11c_f5c3), 0., 0.],
            enable_contact_caching: true,
            merge_events_before_dispatch: true,
            broad_phase_type: 1,
            broad_phase_aabb: [[0.; 4]; 2],
            broad_phase_config: None,
            collision_filter: None,
            shape_tag_codec: None,
            collision_tolerance: f32::from_bits(0x3d4c_cccd),
            relative_collision_accuracy: f32::from_bits(0x3ba3_d70a),
            aabb_margin: f32::from_bits(0x3c23_d70a),
            enable_welding_for_default_objects: false,
            enable_welding_for_critical_objects: false,
            lod: BodyLodConfig::default(),
            enable_sdf_edge_collisions: false,
            enable_collide_work_stealing: true,
            solver,
            solver_enable_post_step: false,
            solver_microsteps: 1,
            max_approach_speed_for_high_quality_solver: 1.,
            enable_deactivation: true,
            enable_penetration_recovery: true,
        };
        info.set_broad_phase_size(2048.);
        info
    }
}

impl WorldInfo {
    /// ca51f0 writes a symmetric cube, including its forced signed W zeros.
    pub fn set_broad_phase_size(&mut self, size: f32) {
        let half = size * 0.5;
        self.broad_phase_aabb = [[-half, -half, -half, -0.], [half, half, half, 0.]];
    }

    /// 0932d0's game overrides of ca4dc0. The authored worldspawn bounds and
    /// actual owner handles are required inputs, independent of render bounds.
    pub fn for_game(bounds: [[f32; 4]; 2], handles: WorldHandles) -> Self {
        let mut info = Self::game_settings(bounds);
        info.persistent_stream_allocator = Some(handles.persistent_stream_allocator);
        info.material_library = Some(handles.material_library);
        info.collision_filter = Some(handles.collision_filter);
        info
    }

    /// Numeric game configuration for Rust-owned world buffers and libraries.
    /// Those owners are passed directly to WorldStorage rather than through
    /// external Cinfo handles; the settings are identical to for_game.
    pub fn game_settings(bounds: [[f32; 4]; 2]) -> Self {
        let mut info = Self::default();
        info.body_buffer_capacity = 0x4000;
        info.motion_buffer_capacity = 0x4000;
        info.constraint_buffer_capacity = 0x4000;
        info.gravity = [0., 0., f32::from_bits(0xc11c_f5c3), 0.];
        info.merge_events_before_dispatch = false;
        info.broad_phase_aabb = bounds.map(|p| [p[0], p[1], p[2], 0.]);
        info.solver_enable_post_step = true;
        info.enable_penetration_recovery = false;
        info
    }
}

#[cfg(test)]
#[path = "info_tests.rs"]
mod tests;
