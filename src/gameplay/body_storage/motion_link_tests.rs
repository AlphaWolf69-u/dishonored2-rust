use super::*;
use crate::gameplay::{
    motion_info::MotionInfo, motion_storage::MotionStorage, space_cells::SpaceCells,
};
#[test]
fn motion_links_match_executed_circular_attachments() {
    for (case, line) in include_str!("../../../tests/fixtures/body_motion_links.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let solver_id = words.next().unwrap();
        let inverse_mass = std::array::from_fn(|_| words.next().unwrap() as u16);
        let count = words.next().unwrap() as usize;
        let flags: Vec<_> = words.by_ref().take(count).collect();
        let mut bodies = BodyStorage::new(count + 1).unwrap();
        let mut motions = MotionStorage::new(2).unwrap();
        let motion_id = motions.reserve().unwrap();
        motions
            .initialize(motion_id, &MotionInfo::default(), &SpaceCells::Single)
            .unwrap();
        let motion = motions.get_mut(motion_id).unwrap();
        motion.solver_id = solver_id;
        motion.inverse_mass = inverse_mass;
        let mut ids = Vec::new();
        for flags in flags {
            let id = bodies.reserve(&mut []).unwrap();
            ids.push(id);
            bodies
                .initialize(
                    id,
                    &BodyInfo {
                        shape: 1,
                        flags,
                        quality_id: 0,
                        ..BodyInfo::default()
                    },
                    true,
                    BodyCreationAssets {
                        shape: ShapeHeader {
                            flags: 0,
                            shape_type: 0,
                            key_bits: 0,
                            dispatch_type: 0,
                            radius: 0.,
                            user_data: 0,
                        },
                        wrapped_shapes: None,
                        name: None,
                        mass_distribution: None,
                    },
                )
                .unwrap();
            bodies.link_motion(id, motion_id, &mut motions).unwrap();
        }
        assert_eq!(
            motions.get(motion_id).unwrap().first_body_id,
            words.next().unwrap(),
            "case {case} first body"
        );
        for id in ids {
            let body = bodies.get(id).unwrap();
            assert_eq!(
                [
                    body.id,
                    body.motion_id,
                    body.flags,
                    body.next_attached_body_id
                ],
                std::array::from_fn(|_| words.next().unwrap()),
                "case {case} body{id:x}"
            );
        }
        assert!(words.next().is_none());
    }
}
