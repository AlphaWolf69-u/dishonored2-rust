use super::*;
use crate::gameplay::{motion_info::MotionInfo, space_cells::SpaceCells};
struct Words<'a>(std::str::SplitWhitespace<'a>);
impl Words<'_> {
    fn word(&mut self) -> u32 {
        u32::from_str_radix(self.0.next().unwrap(), 16).unwrap()
    }
    fn vector(&mut self) -> [f32; 4] {
        std::array::from_fn(|_| f32::from_bits(self.word()))
    }
    fn frame(&mut self) -> PrincipalMassProperties {
        PrincipalMassProperties {
            volume: f32::from_bits(self.word()),
            mass: f32::from_bits(self.word()),
            center: self.vector(),
            inertia: self.vector(),
            orientation: self.vector(),
        }
    }
}
#[test]
fn motion_mass_matches_executed_complete_routine() {
    let mut count = 0;
    for (case, line) in include_str!("../../../tests/fixtures/motion_mass.txt")
        .lines()
        .enumerate()
    {
        let mut input = Words(line.split_whitespace());
        let n = input.word() as usize;
        let order: Vec<u32> = (0..n).map(|_| input.word()).collect();
        let mut bodies = BodyStorage::new((n + 1).max(2)).unwrap();
        let mut frames = Vec::new();
        let mut bounds = Vec::new();
        let mut ids = Vec::new();
        for i in 0..n {
            let id = bodies.reserve(&mut []).unwrap();
            ids.push(id);
            let body = bodies.get_mut(id).unwrap();
            body.shape = i + 1;
            body.transform = std::array::from_fn(|_| input.vector());
            body.motion_to_body_orientation = input.vector();
            body.flags = input.word();
            body.lod_flags = input.word() as u8;
            body.packed_radius = input.word() as u16;
            body.next_attached_body_id =
                order[(order.iter().position(|&v| v == id).unwrap() + 1) % n];
            bounds.push([input.vector(), input.vector()]);
            frames.push(input.frame());
        }
        let center = input.vector();
        let orientation = input.vector();
        let inverse = std::array::from_fn(|_| input.word() as u16);
        let angular = input.vector();
        let threshold = f32::from_bits(input.word());
        let mut motions = MotionStorage::new(2).unwrap();
        let motion_id = motions.reserve().unwrap();
        motions
            .initialize(motion_id, &MotionInfo::default(), &SpaceCells::Single)
            .unwrap();
        let motion = motions.get_mut(motion_id).unwrap();
        motion.first_body_id = order.first().copied().unwrap_or(0xffffff);
        motion.pose.center = center;
        motion.pose.orientation = orientation;
        motion.inverse_mass = inverse;
        motion.angular_velocity = angular;
        let mut observed = Vec::new();
        recompute_motion_mass(
            &mut bodies,
            &mut motions,
            motion_id,
            threshold,
            |_, id| {
                observed.push(id);
                Ok(frames[((id & 0xff_ffff) - 1) as usize])
            },
            |shape, matrix| {
                Ok(crate::gameplay::shape_transform::transform_bounds(matrix, bounds[shape - 1]))
            },
        )
        .unwrap();
        let motion = motions.get(motion_id).unwrap();
        let mut result: Vec<u32> = motion
            .pose
            .center
            .into_iter()
            .chain(motion.pose.orientation)
            .map(f32::to_bits)
            .collect();
        result.extend(motion.inverse_mass.map(u32::from));
        result.extend(motion.angular_velocity.map(f32::to_bits));
        for id in ids {
            let body = bodies.get(id).unwrap();
            result.extend(
                body.transform
                    .into_iter()
                    .flatten()
                    .chain(body.motion_to_body_orientation)
                    .map(f32::to_bits),
            );
            result.extend([u32::from(body.packed_radius), u32::from(body.lod_flags)]);
        }
        let expected: Vec<_> = (0..result.len()).map(|_| input.word()).collect();
        assert_eq!(result, expected, "case {case} output");
        let n = input.word() as usize;
        let expected: Vec<_> = (0..n).map(|_| input.word()).collect();
        assert_eq!(observed, expected, "case {case} mass lookup order");
        assert!(input.0.next().is_none());
        count += 1;
    }
    assert_eq!(count, 256);
}
