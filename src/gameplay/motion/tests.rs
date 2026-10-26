use super::*;
fn snapshot(motion: Motion) -> Vec<u32> {
    motion
        .pose
        .center
        .map(f32::to_bits)
        .into_iter()
        .chain(motion.pose.orientation.map(f32::to_bits))
        .chain([
            u32::from(motion.inverse_mass[0]) | (u32::from(motion.inverse_mass[1]) << 16),
            u32::from(motion.inverse_mass[2]) | (u32::from(motion.inverse_mass[3]) << 16),
            motion.first_body_id,
            motion.solver_id,
            u32::from(motion.linear_velocity_cage[0])
                | (u32::from(motion.linear_velocity_cage[1]) << 16),
            u32::from(motion.linear_velocity_cage[2])
                | (u32::from(motion.integration_factor) << 16),
            u32::from(motion.properties_id) | (u32::from(motion.limits[0]) << 16),
            u32::from(motion.limits[1])
                | (u32::from(motion.cell) << 16)
                | (u32::from(motion.space_splitter_weight) << 24),
        ])
        .chain(motion.linear_velocity.map(f32::to_bits))
        .chain(motion.angular_velocity.map(f32::to_bits))
        .chain(motion.previous_step_linear_velocity.map(f32::to_bits))
        .chain(motion.previous_step_angular_velocity.map(f32::to_bits))
        .collect()
}
#[test]
fn runtime_motion_matches_executed_initializer_and_actual_cell_dispatch() {
    for (case, line) in include_str!("../../../tests/fixtures/motion_runtime.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap() as usize;
        let header = words.next().unwrap();
        let info = MotionInfo {
            properties_id: header as u16,
            enable_deactivation: header >> 16 != 0,
            inverse_mass: f32::from_bits(words.next().unwrap()),
            limits: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            inverse_inertia: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            pose: BodyPose {
                center: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
                orientation: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            },
            linear_velocity: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            angular_velocity: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
        };
        let cells = if count == 1 {
            SpaceCells::Single
        } else {
            SpaceCells::dynamic(count).unwrap()
        };
        assert_eq!(
            snapshot(Motion::initialize(&info, &cells).unwrap()),
            words.by_ref().take(32).collect::<Vec<_>>(),
            "case {case}"
        );
        assert!(words.next().is_none());
    }
}
