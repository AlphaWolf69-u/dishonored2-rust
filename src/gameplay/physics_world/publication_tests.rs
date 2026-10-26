use super::publication::*;
use crate::gameplay::{
    body_activation::ActivationOwners,
    body_quality::{BodyQualityConfig, BodyQualityLibrary},
    body_storage::{activation_snapshot, activation_test_state, BodyActivation},
    broadphase::{BroadphaseQuantization, HybridBroadphase},
    collision_body::SweptBoundsStep,
    constraint_links::ConstraintLinks,
    deactivation::{DeactivationWorld, MotionDeactivation},
    shape_transform::{transform_bounds, ShapeMatrix},
};

#[derive(Default)]
struct Callbacks {
    events: Vec<u32>,
}
impl PublicationCallbacks for Callbacks {
    fn bounds(&mut self, _: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
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
    fn added(&mut self, _: ActivationOwners<'_>, ids: &[u32]) -> Result<(), String> {
        self.events.extend([0xa1, ids.len() as u32]);
        self.events.extend_from_slice(ids);
        Ok(())
    }
}

impl TransformCallbacks for Callbacks {
    fn refresh_transform_cache(&mut self, _: ActivationOwners<'_>, _: u32) -> Result<(), String> {
        panic!("material publication does not change a transform")
    }
    fn update_attached_transform(
        &mut self,
        _: ActivationOwners<'_>,
        _: u32,
        _: i32,
    ) -> Result<(), String> {
        panic!("material publication does not publish an attached transform")
    }
    fn transform_changed(
        &mut self,
        _: ActivationOwners<'_>,
        id: u32,
        static_only: bool,
    ) -> Result<(), String> {
        assert!(!static_only);
        self.events.extend([0xb1, id]);
        Ok(())
    }
}

#[test]
fn publication_matches_complete_executed_bounds_index_activation_and_signals() {
    let qualities = BodyQualityLibrary::new(BodyQualityConfig {
        enable_welding_for_default_objects: false,
        enable_welding_for_critical_objects: false,
        enable_contact_caching: true,
    });
    let quantization =
        BroadphaseQuantization::new([[-100., -100., -100., 0.], [100., 100., 100., 0.]]);
    let step = SweptBoundsStep {
        duration: [0.016; 4],
        rotation_test_scale: 60.,
        contact_distance_scale: [1.; 4],
    };
    let mut count = 0;
    for (row, material_case) in include_str!("../../../tests/fixtures/body_publication.txt")
        .lines()
        .map(|row| (row, false))
        .chain(
            include_str!("../../../tests/fixtures/body_material_publication.txt")
                .lines()
                .map(|row| (row, true)),
        )
    {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let material = material_case.then(|| {
            (
                words.next().unwrap() as u16,
                words.next().unwrap() as i32,
                words.next().unwrap(),
            )
        });
        let case = words.next().unwrap();
        let phase = words.next().unwrap();
        let mode = match words.next().unwrap() {
            0 => BodyAddition::Disabled,
            1 => BodyAddition::Immediate,
            2 => BodyAddition::Queued,
            _ => unreachable!(),
        };
        let activation = if words.next().unwrap() != 0 {
            BodyActivation::KeepDeactivated
        } else {
            BodyActivation::Activate
        };
        let enable_deactivation = words.next().unwrap() != 0;
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
            body.motion_id = words.next().unwrap();
            body.flags = words.next().unwrap();
            body.status = words.next().unwrap() as u16;
            body.next_attached_body_id = words.next().unwrap();
            let position = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
            body.transform = [
                [1., 0., 0., 0.],
                [0., 1., 0., 0.],
                [0., 0., 1., 0.],
                position,
            ];
            body.broadphase_id = u32::MAX;
            body.quality_id = if body.motion_id == 0 { 0 } else { 3 };
            body.packed_radius = 0x3f80;
            body.shape_selectors = [0x80; 8];
            body.motion_to_body_orientation = [0., 0., 0., 1.];
        }
        let n = words.next().unwrap() as usize;
        let ids: Vec<_> = words.by_ref().take(n).collect();
        let mut broadphase = HybridBroadphase::with_body_capacity(32);
        let mut callbacks = Callbacks::default();
        let mut publication = BodyPublication {
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
            enable_deactivation,
            phase,
            step_index: 0,
        };
        publication
            .add(&ids, mode, activation, &mut callbacks)
            .unwrap();
        if let Some((material, mode, target)) = material {
            publication
                .set_material(target, material, mode, &mut callbacks)
                .unwrap();
        }
        let mut actual = activation_snapshot(&bodies, &motions);
        for i in 1..8 {
            let body = bodies.get_by_index(i).unwrap();
            actual.extend(body.bounds.words);
            actual.extend([body.broadphase_id, u32::from(body.max_contact_distance)]);
            actual.extend(bodies.previous_bounds()[i as usize].words);
        }
        actual.extend(
            deactivation
                .states
                .iter()
                .flatten()
                .map(|s| u32::from(s.0[11])),
        );
        if material_case {
            actual.extend(
                (1..8).map(|index| u32::from(bodies.get_by_index(index).unwrap().material_id)),
            );
        }
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
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {case}");
        count += 1;
    }
    assert_eq!(count, 256);
}
