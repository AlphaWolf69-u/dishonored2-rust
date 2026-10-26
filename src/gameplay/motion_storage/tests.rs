use super::*;
pub(crate) fn snapshot(motion: &Motion) -> Vec<u32> {
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
fn motion_storage_matches_executed_startup_and_reservations() {
    for (case, line) in include_str!("../../../tests/fixtures/motion_storage.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let capacity = words.next().unwrap() as usize;
        let mut storage = MotionStorage::new(capacity).unwrap();
        assert_eq!(
            snapshot(storage.get(0).unwrap()),
            words.by_ref().take(32).collect::<Vec<_>>(),
            "case {case} static motion"
        );
        assert_eq!(storage.capacity(), capacity);
        assert_eq!(storage.active_count(), 1);
        assert_eq!(storage.active_ids().next(), None);
        assert!(storage.cell_lists().iter().all(|list| list == &[0]));
        let cells = SpaceCells::dynamic(16).unwrap();
        for _ in 1..capacity {
            let id = storage.reserve().unwrap();
            assert_eq!(id, words.next().unwrap());
            assert_eq!(storage.active_count(), words.next().unwrap());
            assert_eq!(storage.maximum_index(), words.next().unwrap());
            assert_eq!(
                storage.free_head.map_or(0x7fff_ffff, |index| index as u32),
                words.next().unwrap()
            );
            assert!(
                storage.get(id).is_none(),
                "uninitialized record must not expose unspecified floats"
            );
            let header = words.next().unwrap();
            let info = MotionInfo {
                properties_id: header as u16,
                enable_deactivation: header >> 16 != 0,
                inverse_mass: f32::from_bits(words.next().unwrap()),
                limits: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
                inverse_inertia: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
                pose: super::super::rigid_body::BodyPose {
                    center: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
                    orientation: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
                },
                linear_velocity: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
                angular_velocity: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            };
            storage.initialize(id, &info, &cells).unwrap();
            assert_eq!(
                snapshot(storage.get(id).unwrap()),
                words.by_ref().take(32).collect::<Vec<_>>(),
                "case {case} motion {id}"
            );
        }
        assert!(storage.reserve().is_err());
        for i in 1..capacity {
            if i % 3 != 0 {
                storage.motions[i].space_splitter_weight = 0;
            }
        }
        assert_eq!(
            storage.active_ids().next().unwrap_or(0x7fff_ffff),
            words.next().unwrap()
        );
        assert!(words.next().is_none());
    }
}
