use super::*;
use crate::gameplay::{
    body_quality::BodyQualityConfig,
    body_storage::{activation_snapshot, activation_test_state},
    broadphase::BroadphaseQuantization,
    collision_user_data::velocity::{
        publish_velocities, CollisionVelocities, VelocityDataStore, VelocityPublication,
    },
    deactivation::{DeactivationWorld, MotionDeactivation},
    motion_properties::MotionPropertiesLibrary,
    shape_transform::transform_bounds,
};

#[derive(Default)]
struct Callbacks {
    events: Vec<u32>,
    reference: u32,
    data: Vec<CollisionVelocities>,
}
impl VelocityDataStore for Callbacks {
    type Reference = usize;
    fn referenced_body(&self, reference: usize) -> Result<u32, String> {
        assert_eq!(reference, 0);
        Ok(self.reference)
    }
    fn velocities(&mut self, user_data: u64) -> Result<&mut CollisionVelocities, String> {
        self.data
            .get_mut(user_data as usize)
            .ok_or("missing collision payload".into())
    }
}
impl PublicationCallbacks for Callbacks {
    fn bounds(&mut self, _: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        self.events.push(7);
        Ok(transform_bounds(
            frame,
            [[-2., -2., -2., 0.], [2., 2., 2., 0.]],
        ))
    }
    fn activation(
        &mut self,
        _: ActivationOwners<'_>,
        ids: &[u32],
        active: bool,
    ) -> Result<(), String> {
        self.events
            .extend([if active { 0xa0 } else { 0xa2 }, ids.len() as u32]);
        self.events.extend_from_slice(ids);
        Ok(())
    }
    fn added(&mut self, _: ActivationOwners<'_>, _: &[u32]) -> Result<(), String> {
        panic!("unexpected body addition")
    }
}
impl TransformCallbacks for Callbacks {
    fn refresh_transform_cache(&mut self, _: ActivationOwners<'_>, _: u32) -> Result<(), String> {
        panic!("unexpected transform cache refresh")
    }
    fn update_attached_transform(
        &mut self,
        _: ActivationOwners<'_>,
        _: u32,
        _: i32,
    ) -> Result<(), String> {
        panic!("unexpected pose update")
    }
    fn transform_changed(
        &mut self,
        _: ActivationOwners<'_>,
        body: u32,
        static_only: bool,
    ) -> Result<(), String> {
        assert!(!static_only);
        self.events.extend([0xa3, body]);
        Ok(())
    }
}

#[test]
fn velocities_and_ring_bounds_match_complete_executed_world_operations() {
    let quantization =
        BroadphaseQuantization::new([[-100., -100., -100., 0.], [100., 100., 100., 0.]]);
    let step = SweptBoundsStep {
        duration: [0.016; 4],
        rotation_test_scale: 60.,
        contact_distance_scale: [1.; 4],
    };
    let mut cases = 0;
    for row in include_str!("../../../../../tests/fixtures/world_body_velocity.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let case = words.next().unwrap();
        let phase = words.next().unwrap();
        let mode = words.next().unwrap() as i32;
        let preactive = words.next().unwrap() != 0;
        let qbits = words.next().unwrap();
        let qid = words.next().unwrap() as u8;
        let qualities = BodyQualityLibrary::new(BodyQualityConfig {
            enable_welding_for_default_objects: qbits & 1 != 0,
            enable_welding_for_critical_objects: qbits & 2 != 0,
            enable_contact_caching: qbits & 4 != 0,
        });
        let (mut bodies, mut motions) = activation_test_state(&mut words);
        let states = (0..4)
            .map(|i| {
                let mut state = MotionDeactivation::new(i, words.next().unwrap() as u8);
                state.0[11] = 17;
                state
            })
            .collect();
        let mut deactivation = DeactivationWorld::new(states);
        for i in 1..8 {
            let id = bodies.get_by_index(i).unwrap().id;
            let body = bodies.get_mut(id).unwrap();
            body.transform = [
                [1., 0., 0., 0.],
                [0., 1., 0., 0.],
                [0., 0., 1., 0.],
                [i as f32, 0., 0., 0.],
            ];
            body.quality_id = qid;
            body.user_data = i as u64;
            body.packed_radius = 0x3f80;
            body.shape_selectors = [0x80; 8];
            body.motion_to_body_orientation = [0., 0., 0., 1.];
        }
        for i in 1..4 {
            let motion = motions.get_mut(i).unwrap();
            motion.pose.center = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            motion.pose.orientation =
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            motion.linear_velocity = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            motion.angular_velocity =
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        }
        let target = bodies.get_by_index(1).unwrap().id;
        if case % 13 == 0 {
            bodies.get_mut(target).unwrap().flags &= !2;
        }
        if preactive {
            bodies.activate_attached(target, &mut motions).unwrap();
        }
        let linear = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let angular = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let mut broadphase = HybridBroadphase::with_body_capacity(32);
        let mut callbacks = Callbacks::default();
        callbacks.reference = target;
        callbacks.data.resize(8, CollisionVelocities::default());
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
            enable_deactivation: true,
            phase,
            step_index: 0,
        };
        if mode == 0 {
            let mut commands =
                crate::gameplay::physics_world::commands::PhysicsCommands::new().unwrap();
            let mut properties = MotionPropertiesLibrary::<u32, u32>::default();
            properties.initialize(9.81, |_, _, _, _| {}).unwrap();
            let deferred = case & 128 != 0;
            publish_velocities(
                &mut VelocityPublication {
                    world: &mut world,
                    callbacks: &mut callbacks,
                    properties: &properties,
                    commands: &commands,
                    grow_one_at_a_time: false,
                },
                0,
                linear,
                angular,
                deferred,
            )
            .unwrap();
            if deferred {
                commands
                    .dispatch(
                        &mut world,
                        &crate::gameplay::space_cells::SpaceCells::Single,
                        &mut callbacks,
                    )
                    .unwrap();
            }
        } else {
            world
                .set_velocity(target, linear, angular, mode, &mut callbacks)
                .unwrap();
        }
        let mut actual = activation_snapshot(&bodies, &motions);
        for i in 1..4 {
            let motion = motions.get(i).unwrap();
            actual.extend(
                motion
                    .linear_velocity
                    .into_iter()
                    .chain(motion.angular_velocity)
                    .map(f32::to_bits),
            );
        }
        for i in 1..8 {
            let body = bodies.get_by_index(i).unwrap();
            actual.extend(body.bounds.words);
            actual.push(u32::from(body.max_contact_distance));
        }
        actual.extend(deactivation.states.iter().flatten().map(|s| u32::from(s.0[11])));
        actual.push(deactivation.pending_bodies.len() as u32);
        actual.extend(&deactivation.pending_bodies);
        actual.push(deactivation.body_commands.len() as u32);
        for command in &deactivation.body_commands {
            actual.extend([command.body, u32::from(command.activated)]);
        }
        actual.push(broadphase.pending_body_bits().words()[0]);
        actual.extend(broadphase.pending_body_range().unwrap_or([u32::MAX, 0]));
        actual.push(callbacks.events.len() as u32);
        actual.extend(callbacks.events);
        assert_eq!(actual, words.collect::<Vec<_>>(), "case{case}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
