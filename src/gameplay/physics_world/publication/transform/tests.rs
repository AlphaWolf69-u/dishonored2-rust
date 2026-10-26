use super::*;
use crate::gameplay::{
    body_quality::BodyQualityConfig, broadphase::BroadphaseQuantization,
    deactivation::DeactivationWorld, motion_info::MotionInfo, motion_storage::MotionStorage,
};

struct Callbacks {
    case: u32,
    events: Vec<u32>,
}
impl PublicationCallbacks for Callbacks {
    fn bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        assert_eq!(shape, 7);
        self.events.push(7);
        Ok(crate::gameplay::shape_transform::transform_bounds(
            frame,
            [[-2., -2., -2., 0.], [2., 2., 2., 0.]],
        ))
    }
    fn activation(&mut self, _: ActivationOwners<'_>, _: &[u32], _: bool) -> Result<(), String> {
        panic!("unexpected activation boundary")
    }
    fn added(&mut self, _: ActivationOwners<'_>, _: &[u32]) -> Result<(), String> {
        panic!("unexpected addition boundary")
    }
}
impl TransformCallbacks for Callbacks {
    fn refresh_transform_cache(
        &mut self,
        owners: ActivationOwners<'_>,
        id: u32,
    ) -> Result<(), String> {
        self.events.push(2);
        if self.case & 16 != 0 {
            owners.bodies.get_mut(id).unwrap().packed_radius = 0x4000;
        }
        Ok(())
    }
    fn update_attached_transform(
        &mut self,
        owners: ActivationOwners<'_>,
        id: u32,
        mode: i32,
    ) -> Result<(), String> {
        self.events.extend([3, mode as u32]);
        owners.bodies.get_mut(id).unwrap().status ^= 0x400;
        Ok(())
    }
    fn transform_changed(
        &mut self,
        _: ActivationOwners<'_>,
        _: u32,
        static_only: bool,
    ) -> Result<(), String> {
        self.events.push(if static_only { 5 } else { 6 });
        Ok(())
    }
}

#[test]
fn shared_world_transform_matches_complete_executed_static_and_dynamic_paths() {
    let quantization =
        BroadphaseQuantization::new([[-100., -100., -100., 0.], [100., 100., 100., 0.]]);
    let qualities = BodyQualityLibrary::new(BodyQualityConfig::default());
    let mut cases = 0;
    for row in include_str!("../../../../../tests/fixtures/world_body_transform.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let case = w[0];
        let cells = if case & 32 != 0 {
            SpaceCells::Dynamic(vec![[-1e6; 4], [0.; 4], [-1e6; 4], [-1e6; 4]])
        } else {
            SpaceCells::Single
        };
        let mode = w[1] as i32;
        let mut bodies = BodyStorage::new(2).unwrap();
        let id = bodies.reserve(&mut []).unwrap();
        assert_eq!(id, 0x01000001);
        let body = bodies.get_mut(id).unwrap();
        body.transform = std::array::from_fn(|i| vector(2 + i * 4));
        body.motion_id = case & 1;
        body.flags = if case & 1 != 0 {
            2 | if case & 64 != 0 { 8 } else { 0 }
        } else {
            1
        };
        body.status = w[46] as u16;
        body.tim_angle = w[47] as u16;
        body.max_tim_distance = w[48] as u16;
        body.angular_tim_distance = w[49] as u16;
        body.packed_radius = w[50] as u16;
        body.broadphase_id = if case & 2 != 0 { 0 } else { u32::MAX };
        body.bounds = PackedBodyBounds { words: [0; 4] };
        body.shape = 7;
        body.next_attached_body_id = id;
        body.shape_selectors = [0x80; 8];
        body.motion_to_body_orientation = vector(42);
        let mut motions = MotionStorage::new(2).unwrap();
        let motion = motions.reserve().unwrap();
        motions
            .initialize(motion, &MotionInfo::default(), &cells)
            .unwrap();
        let motion = motions.get_mut(motion).unwrap();
        motion.pose.center = vector(34);
        motion.pose.orientation = vector(38);
        motion.first_body_id = id;
        motion.cell = 0;
        let mut broadphase = HybridBroadphase::with_body_capacity(2);
        let mut deactivation = DeactivationWorld::new(Vec::new());
        let step = SweptBoundsStep {
            duration: [0.; 4],
            rotation_test_scale: 0.,
            contact_distance_scale: vector(51),
        };
        let mut callbacks = Callbacks {
            case,
            events: Vec::new(),
        };
        // Record selection alongside the callback trace; the Single splitter
        // is the real zero-cell policy used by the source callback boundary.
        let mut world = BodyPublication {
            bodies: &mut bodies,
            motions: &mut motions,
            deactivation: &mut deactivation,
            broadphase: &mut broadphase,
            packing: &quantization.packed,
            qualities: &qualities,
            constraints: ConstraintLinks {
                heads: &[],
                constraints: &[],
                next: &[],
            },
            tolerance: [0.05, 0.05, 0.05, 0.],
            step: &step,
            enable_deactivation: case & 4 != 0,
            phase: 0,
            step_index: 0,
        };
        let frame = std::array::from_fn(|i| vector(18 + i * 4));
        if case & 128 != 0 {
            let mut commands =
                crate::gameplay::physics_world::commands::PhysicsCommands::new().unwrap();
            commands
                .enqueue_transform(id, frame, mode as u8, false)
                .unwrap();
            commands
                .dispatch(&mut world, &cells, &mut callbacks)
                .unwrap();
        } else {
            world
                .set_transform(id, frame, mode, &cells, &mut callbacks)
                .unwrap();
        }
        let body = bodies.get(id).unwrap();
        let motion = motions.get(1).unwrap();
        let mut actual: Vec<_> = body
            .transform
            .into_iter()
            .flatten()
            .chain(body.motion_to_body_orientation)
            .map(f32::to_bits)
            .collect();
        actual.extend([
            u32::from(body.status),
            u32::from(body.tim_angle),
            u32::from(body.max_tim_distance),
            u32::from(body.angular_tim_distance),
        ]);
        actual.extend(body.bounds.words);
        actual.push(u32::from(body.packed_radius));
        actual.extend(
            motion
                .pose
                .center
                .into_iter()
                .chain(motion.pose.orientation)
                .map(f32::to_bits),
        );
        actual.extend([
            u32::from(motion.cell),
            u32::from(bodies.allocation_marks()[1]),
        ]);
        assert_eq!(actual, w[55..94], "state case{case}");
        // Preserve mode0/1 values following event3; only standalone selection
        // and mark events are internal operations rather than owner callbacks.
        let mut expected = Vec::new();
        let mut source = w[95..].iter().copied();
        while let Some(event) = source.next() {
            if event != 1 && event != 4 {
                expected.push(event);
            }
            if event == 3 {
                expected.push(source.next().unwrap());
            }
        }
        assert_eq!(callbacks.events, expected, "callbacks case{case}");
        assert_eq!(
            broadphase.pending_body_range(),
            w[95..].contains(&4).then_some([1, 1]),
            "broadphase case{case}"
        );
        cases += 1;
    }
    assert_eq!(cases, 512);
}
