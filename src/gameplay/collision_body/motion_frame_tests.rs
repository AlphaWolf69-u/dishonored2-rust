use super::*;
#[test]
fn attached_body_frame_radius_and_speed_match_executed_routines() {
    for (case, line) in include_str!("../../../tests/fixtures/body_motion_frame.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let provided = words.next().unwrap() != 0;
        let orientation = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let mut body = CollisionBody {
            transform: std::array::from_fn(|_| {
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
            }),
            ..CollisionBody::default()
        };
        let motion = BodyPose {
            center: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            orientation: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
        };
        let bounds =
            std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())));
        body.attach_motion_frame(&motion, provided.then_some(orientation));
        let actual: Vec<_> = body
            .transform
            .into_iter()
            .flatten()
            .chain(body.motion_to_body_orientation)
            .map(f32::to_bits)
            .collect();
        assert_eq!(
            actual,
            words.by_ref().take(20).collect::<Vec<_>>(),
            "case {case} relative frame"
        );
        body.update_motion_radius(motion.center, bounds);
        assert_eq!(
            u32::from(body.packed_radius),
            words.next().unwrap(),
            "case {case} radius"
        );
        let velocity = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let threshold = f32::from_bits(words.next().unwrap());
        let reference = f32::from_bits(words.next().unwrap());
        body.lod_flags = words.next().unwrap() as u8;
        body.packed_average_surface_velocity = words.next().unwrap() as u16;
        body.update_speed_lod(velocity, threshold, reference);
        assert_eq!(
            u32::from(body.lod_flags),
            words.next().unwrap(),
            "case {case} speed flags"
        );
        assert_eq!(
            u32::from(body.packed_average_surface_velocity),
            words.next().unwrap(),
            "case {case} retained reference"
        );
        assert!(words.next().is_none());
    }
}
