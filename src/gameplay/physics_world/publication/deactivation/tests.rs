use super::*;
use crate::gameplay::{
    body_quality::BodyQualityConfig,
    body_storage::{activation_snapshot, activation_test_state},
    broadphase::BroadphaseQuantization,
    collision_body::BodyCacheSettings,
    deactivation::{DeactivatedIsland, MotionDeactivation},
    physics_world::{PublicationContext, PublicationEvents, PublicationGeometry},
};

#[derive(Default)]
struct Callbacks {
    events: Vec<u32>,
}
impl PublicationEvents for Callbacks {
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
    fn transform_changed(
        &mut self,
        _: ActivationOwners<'_>,
        _: u32,
        _: bool,
    ) -> Result<(), String> {
        panic!("attached update must not emit transform signals")
    }
}
struct NoGeometry;
impl PublicationGeometry for NoGeometry {
    fn is_null(&mut self, _: usize) -> Result<bool, String> {
        panic!("empty preparation must not resolve a shape")
    }
    fn bounds(&mut self, _: usize, _: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        panic!("empty preparation must not query geometry")
    }
}

#[test]
fn attached_transform_matches_complete_executed_deactivation_update() {
    let packing = BroadphaseQuantization::new([[-100.; 4], [100.; 4]]);
    let qualities = BodyQualityLibrary::new(BodyQualityConfig::default());
    let step = SweptBoundsStep {
        duration: [0.; 4],
        rotation_test_scale: 0.,
        contact_distance_scale: [0.; 4],
    };
    let mut cases = 0;
    for row in include_str!("../../../../../tests/fixtures/attached_transform.txt").lines() {
        let mut w = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let case = w.next().unwrap();
        let phase = w.next().unwrap();
        let mode = w.next().unwrap() as i32;
        let enabled = w.next().unwrap() != 0;
        let preactive = w.next().unwrap() != 0;
        let is_static = w.next().unwrap() != 0;
        let suppress = w.next().unwrap() != 0;
        let target = w.next().unwrap();
        let (mut bodies, mut motions) = activation_test_state(&mut w);
        let states = (0..4)
            .map(|i| {
                let mut state = MotionDeactivation::new(i, w.next().unwrap() as u8);
                state.0[11] = 17;
                state.set_island(w.next().unwrap() as i32);
                state
            })
            .collect();
        let n = w.next().unwrap() as usize;
        let members = w.by_ref().take(n).collect();
        let wake = w.next().unwrap() as u8;
        let queued = w.next().unwrap() as u8;
        let mut deactivation = DeactivationWorld::new(states);
        deactivation.islands.push(Some(DeactivatedIsland {
            id: 0,
            bodies: members,
            wake,
            queued,
            ..Default::default()
        }));
        deactivation.suppress_island_detach = suppress;
        if queued != 0 {
            deactivation.activation_queue.push(0);
        }
        if preactive {
            bodies.activate_attached(target, &mut motions).unwrap();
        }
        if is_static {
            bodies.get_mut(target).unwrap().flags |= 1;
        }
        let mut broadphase = HybridBroadphase::with_body_capacity(32);
        let mut callbacks = Callbacks::default();
        PublicationContext {
            geometry: &mut NoGeometry,
            events: &mut callbacks,
            cache: BodyCacheSettings {
                bounds_tolerance: [0.; 4],
                cache_margin: 0.,
                lod: None,
                packing: &packing.packed,
                step: &step,
            },
            qualities: &qualities,
            constraints: ConstraintLinks {
                heads: &[],
                constraints: &[],
                next: &[],
            },
            enable_deactivation: enabled,
            phase,
            step_index: 0,
        }
        .update_attached_transform(
            ActivationOwners {
                bodies: &mut bodies,
                motions: &mut motions,
                deactivation: &mut deactivation,
                broadphase: &mut broadphase,
            },
            target,
            mode,
        )
        .unwrap();
        let mut actual = activation_snapshot(&bodies, &motions);
        for state in deactivation.states.iter().flatten() {
            actual.extend([
                u32::from(state.0[14]),
                u32::from(state.0[11]),
                state.island() as u32,
            ]);
        }
        let island = deactivation.islands[0].as_ref().unwrap();
        for values in [
            &island.bodies,
            &deactivation.activation_queue,
            &deactivation.pending_bodies,
        ] {
            actual.push(values.len() as u32);
            actual.extend(values);
        }
        actual.extend([u32::from(island.wake), u32::from(island.queued)]);
        actual.push(deactivation.body_commands.len() as u32);
        for command in &deactivation.body_commands {
            actual.extend([command.body, u32::from(command.activated)]);
        }
        actual.push(broadphase.pending_body_bits().words()[0]);
        actual.extend(broadphase.pending_body_range().unwrap_or([u32::MAX, 0]));
        actual.push(callbacks.events.len() as u32);
        actual.extend(callbacks.events);
        assert_eq!(actual, w.collect::<Vec<_>>(), "case{case}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
