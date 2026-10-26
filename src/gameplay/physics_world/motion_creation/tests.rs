use super::*;
use crate::gameplay::{
    body_quality::{BodyQualityConfig, BodyQualityLibrary},
    body_storage::BodyStorage,
    broadphase::{BroadphaseQuantization, HybridBroadphase},
    collision_body::{BodyCacheSettings, SweptBoundsStep},
    component_state::pool::ComponentPool,
    constraint_links::ConstraintLinks,
    deactivation::DeactivationWorld,
    motion_storage::{motion_snapshot, MotionStorage},
    physics_world::{
        ComponentPublicationEvents, PublicationContext, PublicationEvent, PublicationGeometry,
        PublicationSignals,
    },
    rigid_body::BodyPose,
    shape_transform::ShapeMatrix,
    signal::Signal,
    trigger_body::{DeferredPhysicsState, TriggerComponent, TriggerComponentStore},
    water_component::WaterComponent,
};

struct NoGeometry;
impl PublicationGeometry for NoGeometry {
    fn is_null(&mut self, _: usize) -> Result<bool, String> {
        panic!("motion creation must not query geometry")
    }
    fn bounds(&mut self, _: usize, _: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        panic!("motion creation must not query geometry")
    }
}

fn info(w: &[u32]) -> MotionInfo {
    let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
    MotionInfo {
        properties_id: w[0] as u16,
        enable_deactivation: (w[0] >> 16) & 255 != 0,
        inverse_mass: f32::from_bits(w[1]),
        limits: [f32::from_bits(w[2]), f32::from_bits(w[3])],
        inverse_inertia: vector(4),
        pose: BodyPose {
            center: vector(8),
            orientation: vector(12),
        },
        linear_velocity: vector(16),
        angular_velocity: vector(20),
    }
}

#[test]
fn motion_creation_matches_complete_executed_manager_states_and_observers() {
    let lines: Vec<_> = include_str!("../../../../tests/fixtures/world_motion_creation.txt")
        .lines()
        .collect();
    let packing = BroadphaseQuantization::new([[-100.; 4], [100.; 4]]);
    let qualities = BodyQualityLibrary::new(BodyQualityConfig::default());
    let step = SweptBoundsStep {
        duration: [0.; 4],
        rotation_test_scale: 0.,
        contact_distance_scale: [0.; 4],
    };
    let cells = SpaceCells::dynamic(16).unwrap();
    let mut count = 0;
    for group in lines.chunks_exact(16) {
        let mut motions = None;
        let mut deactivation = None;
        let mut bodies = BodyStorage::new(2).unwrap();
        let mut broadphase = HybridBroadphase::with_body_capacity(2);
        let mut triggers = ComponentPool::<TriggerComponent>::new(0, 0xd8).unwrap();
        let mut waters = ComponentPool::<WaterComponent>::new(0, 0x40).unwrap();
        let mut actors = crate::gameplay::physics_actor::PhysicsActorLinks {
            actors: crate::gameplay::physics_actor::test_actor_pool(Vec::new()),
            bodies: Vec::new(),
        };
        let mut deferred = DeferredPhysicsState {
            world_flags: [0; 2],
            component_flag: 0,
        };
        for row in group {
            let w: Vec<_> = row
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let case = w[0];
            if motions.is_none() {
                motions = Some(MotionStorage::new(w[1] as usize).unwrap());
                deactivation = Some(DeactivationWorld::for_motion_capacity(w[2] as usize).unwrap());
                let states = deactivation.as_ref().unwrap();
                assert_eq!(states.state(0).unwrap().island(), -1);
                assert!(states.states.iter().skip(1).all(Option::is_none));
            }
            let motions = motions.as_mut().unwrap();
            let deactivation = deactivation.as_mut().unwrap();
            let input = info(&w[3..27]);
            let mut trace = Vec::new();
            let id;
            {
                let mut world = BodyPublication {
                    bodies: &mut bodies,
                    motions,
                    deactivation,
                    broadphase: &mut broadphase,
                    packing: &packing.packed,
                    qualities: &qualities,
                    constraints: ConstraintLinks {
                        heads: &[],
                        constraints: &[],
                        next: &[],
                    },
                    tolerance: [0.; 4],
                    step: &step,
                    enable_deactivation: true,
                    phase: 0,
                    step_index: 0,
                };
                let mut signals = PublicationSignals::<u32, u32>::default();
                signals.motion_created.subscribe(1, 2);
                let dispatch = |_: &mut Signal<u32, u32>,
                                s,
                                c,
                                owners: ActivationOwners<'_>,
                                event: PublicationEvent<'_>,
                                _: &mut TriggerComponentStore<'_>| {
                    assert_eq!((s, c), (1, 2));
                    let PublicationEvent::MotionCreated { info, motion: id } = event else {
                        panic!("unexpected creation signal");
                    };
                    assert_eq!(info.enable_deactivation, input.enable_deactivation);
                    let state = owners.deactivation.state(id).unwrap();
                    let motion = owners.motions.get(id).unwrap();
                    trace.extend([
                        id,
                        u32::from(state.mode()),
                        state.island() as u32,
                        u32::from(motion.properties_id),
                        motion.pose.center[0].to_bits(),
                    ]);
                    for i in 0..=id {
                        let state = owners.deactivation.state_mut(i).unwrap();
                        for at in [
                            8, 9, 12, 13, 15, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
                        ] {
                            state.0[at] = (case * 7 + i * 13 + at as u32 * 3) as u8;
                        }
                    }
                    owners.deactivation.state_mut(id).unwrap().0[11] =
                        if id % 3 != 0 { 17 } else { 255 };
                    let motion = owners.motions.get_mut(id).unwrap();
                    motion.pose.center[0] = crate::float::add(motion.pose.center[0], 0.25);
                    Ok(())
                };
                let mut events = ComponentPublicationEvents {
                    components: TriggerComponentStore {
                        trigger_class: 17,
                        water_class: 23,
                        triggers: &mut triggers,
                        waters: &mut waters,
                        actors: &mut actors,
                        deferred: &mut deferred,
                    },
                    signals: &mut signals,
                    dispatch,
                };
                let mut geometry = NoGeometry;
                let mut callbacks = PublicationContext {
                    geometry: &mut geometry,
                    events: &mut events,
                    cache: BodyCacheSettings {
                        bounds_tolerance: [0.; 4],
                        cache_margin: 0.,
                        lod: None,
                        step: &step,
                        packing: &packing.packed,
                    },
                    qualities: &qualities,
                    constraints: ConstraintLinks {
                        heads: &[],
                        constraints: &[],
                        next: &[],
                    },
                    enable_deactivation: true,
                    phase: 0,
                    step_index: 0,
                };
                id = world.create_motion(&input, &cells, &mut callbacks).unwrap();
            }
            let mut actual = vec![
                id,
                motions.active_count(),
                motions.maximum_index(),
                u32::from(motions.has_free_slot()),
                deactivation.states.len() as u32,
                deactivation.states.capacity() as u32,
            ];
            for i in 1..=id {
                actual.extend(motion_snapshot(motions.get(i).unwrap()));
            }
            for i in 0..=id {
                let state = deactivation.state(i).unwrap();
                actual.extend(
                    state
                        .0
                        .chunks_exact(4)
                        .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
                );
            }
            assert!(deactivation
                .states
                .iter()
                .skip(id as usize + 1)
                .all(Option::is_none));
            actual.push(trace.len() as u32);
            actual.extend(trace);
            assert_eq!(actual, w[27..], "case{case} id{id}");
            count += 1;
        }
    }
    assert_eq!(count, 512);
}
