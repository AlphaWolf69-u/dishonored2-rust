use super::*;
use crate::gameplay::{
    body_activation::ActivationOwners,
    body_quality::{BodyQualityConfig, BodyQualityLibrary},
    body_storage::BodyStorage,
    broadphase::{BroadphaseQuantization, HybridBroadphase},
    collision_body::{BodyCacheSettings, SweptBoundsStep},
    collision_user_data::{velocity::CollisionVelocities, CollisionUserData},
    component_state::pool::ComponentPool,
    constraint_links::ConstraintLinks,
    deactivation::DeactivationWorld,
    motion_info::MotionInfo,
    motion_properties::MotionProfile,
    motion_storage::MotionStorage,
    physics_world::{ComponentPublicationEvents, PublicationEvent, PublicationSignals},
    reference_frame::ReferenceFrame,
    shape_transform::transform_bounds,
    signal::Signal,
    trigger_body::{
        transform::publish_transform, DeferredPhysicsState, TriggerComponent, TriggerComponentStore,
    },
    water_component::WaterComponent,
};

struct Geometry;
const BOUNDS: [[[f32; 4]; 2]; 2] = [
    [[-2., -3., -4., 0.], [2., 3., 4., 0.]],
    [[-1., -2., -3., 0.], [1., 2., 3., 0.]],
];
impl PublicationGeometry for Geometry {
    fn is_null(&mut self, shape: usize) -> Result<bool, String> {
        BOUNDS.get(shape).ok_or("missing shape")?;
        Ok(false)
    }
    fn bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        Ok(transform_bounds(
            frame,
            *BOUNDS.get(shape).ok_or("missing shape")?,
        ))
    }
}
fn vector(words: &mut impl Iterator<Item = u32>) -> [f32; 4] {
    std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
}
fn payload(words: &mut impl Iterator<Item = u32>) -> CollisionVelocities {
    CollisionVelocities {
        scaled_linear: vector(words),
        scaled_angular: vector(words),
        requested_linear: vector(words),
        requested_angular: vector(words),
    }
}

#[test]
fn typed_trigger_publication_matches_complete_executed_physics_queue_and_water_pipeline() {
    let packing = BroadphaseQuantization::new([[-100.; 4], [100.; 4]]);
    let qualities = BodyQualityLibrary::new(BodyQualityConfig::default());
    let step = SweptBoundsStep {
        duration: [0.016; 4],
        rotation_test_scale: 60.,
        contact_distance_scale: [1.; 4],
    };
    let cells = SpaceCells::Single;
    let first = ComponentHandle {
        class_id: 17,
        slot: 0,
    };
    let water_handles = std::array::from_fn::<_, 2, _>(|i| ComponentHandle {
        class_id: 23,
        slot: i as u16,
    });
    let mut cases = 0;
    for row in include_str!("../../../../tests/fixtures/trigger_publication.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let case = words.next().unwrap();
        let mut deferred = DeferredPhysicsState {
            world_flags: std::array::from_fn(|_| words.next().unwrap() as u8),
            component_flag: words.next().unwrap() as u8,
        };
        let clear = words.next().unwrap() != 0;
        let initial_water = words.next().unwrap() as i32;
        let scale = words.next().unwrap();
        let water_flags: [u32; 2] = std::array::from_fn(|_| words.next().unwrap());
        let statuses: [u16; 2] = std::array::from_fn(|_| words.next().unwrap() as u16);
        let old_frames: [ShapeMatrix; 2] =
            std::array::from_fn(|_| std::array::from_fn(|_| vector(&mut words)));
        let new_frame: ShapeMatrix = std::array::from_fn(|_| vector(&mut words));
        let initial_payloads = std::array::from_fn::<_, 2, _>(|_| payload(&mut words));
        let mut bodies = BodyStorage::new(3).unwrap();
        let mut motions = MotionStorage::new(3).unwrap();
        for i in 1..=2 {
            let id = motions.reserve().unwrap();
            assert_eq!(id, i);
            motions
                .initialize(id, &MotionInfo::default(), &cells)
                .unwrap();
            motions.get_mut(id).unwrap().first_body_id = 0x01000000 + i;
        }
        let mut deactivation = DeactivationWorld::new(Vec::new());
        let mut broadphase = HybridBroadphase::with_body_capacity(3);
        let mut triggers = ComponentPool::<TriggerComponent>::new(0, 0xd8).unwrap();
        let mut waters = ComponentPool::<WaterComponent>::new(0, 0x40).unwrap();
        let mut actors = crate::gameplay::physics_actor::PhysicsActorLinks {
            actors: crate::gameplay::physics_actor::test_actor_pool(Vec::new()),
            bodies: Vec::new(),
        };
        for i in 0..2 {
            let id = bodies.reserve(&mut []).unwrap();
            assert_eq!(id, 0x01000001 + i as u32);
            let body = bodies.get_mut(id).unwrap();
            body.transform = old_frames[i];
            let dynamic = case >= 512 && (i == 0 || case & 256 != 0);
            body.flags = if dynamic { 2 } else { 1 };
            body.motion_id = if dynamic { i as u32 + 1 } else { 0 };
            body.quality_id = if dynamic { 3 } else { 0 };
            body.packed_radius = if dynamic { 0x3f80 } else { 0 };
            body.status = statuses[i];
            body.shape = i;
            body.next_attached_body_id = id;
            body.broadphase_id = u32::MAX;
            body.shape_selectors = [0x80; 8];
            body.motion_to_body_orientation = [0., 0., 0., 1.];
            let slot = triggers
                .allocate(|_| TriggerComponent::new(&mut actors.bodies))
                .unwrap();
            assert_eq!(slot, i as u16);
            let trigger = triggers.get_mut(slot).unwrap();
            trigger.velocities = initial_payloads[i];
            actors.bodies[trigger.movement_body].body_id = id;
            body.user_data = CollisionUserData {
                component: ComponentHandle { class_id: 17, slot },
            }
            .encode();
            let slot = waters.allocate(|_| Ok(WaterComponent::default())).unwrap();
            let water = waters.get_mut(slot).unwrap();
            water.owner.flags = 8;
            water
                .update_transform(body, body.transform, |shape, frame| {
                    Ok(transform_bounds(frame, BOUNDS[shape]))
                })
                .unwrap();
            water.owner.flags = water_flags[i];
        }
        triggers.get_mut(0).unwrap().water =
            (initial_water >= 0).then(|| water_handles[initial_water as usize]);
        let mut properties = MotionPropertiesLibrary::<u32, u32>::default();
        let mut p = [0; 16];
        p[0] = 1;
        p[3] = scale;
        properties
            .add(MotionProfile::from_words(p), |_, _, _, _| {})
            .unwrap();
        properties
            .add(MotionProfile::from_words(p), |_, _, _, _| {})
            .unwrap();
        let mut commands = PhysicsCommands::new().unwrap();
        let mut trace = Vec::new();
        {
            let mut world = BodyPublication {
                bodies: &mut bodies,
                motions: &mut motions,
                deactivation: &mut deactivation,
                broadphase: &mut broadphase,
                packing: &packing.packed,
                qualities: &qualities,
                constraints: ConstraintLinks {
                    heads: &[],
                    constraints: &[],
                    next: &[],
                },
                tolerance: [0.05, 0.05, 0.05, 0.],
                step: &step,
                enable_deactivation: false,
                phase: 0,
                step_index: 0,
            };
            let mut signals = PublicationSignals::<u32, u32>::default();
            signals.static_transform.subscribe(0, 5);
            signals.transform.subscribe(0, 6);
            let dispatch = |_: &mut Signal<u32, u32>,
                            subscriber,
                            callback,
                            _: ActivationOwners<'_>,
                            event: PublicationEvent<'_>,
                            components: &mut TriggerComponentStore<'_>| {
                assert_eq!(subscriber, 0);
                let PublicationEvent::Transform { body, static_only } = event else {
                    panic!("unexpected world event");
                };
                assert_eq!(callback, if static_only { 5 } else { 6 });
                trace.extend([callback, body]);
                if static_only && case & 16 != 0 {
                    let movement = components.trigger(first)?.movement_body;
                    components.actors.bodies[movement].body_id = 0x01000002;
                }
                if !static_only && case & 32 != 0 {
                    components.trigger_mut(first)?.water = None;
                }
                if static_only && case & 64 != 0 {
                    components.trigger_mut(first)?.water = Some(water_handles[1]);
                }
                if !static_only && case & 128 != 0 {
                    components.deferred.component_flag ^= 1;
                }
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
            let mut geometry = Geometry;
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
                enable_deactivation: false,
                phase: 0,
                step_index: 0,
            };
            let pose = ReferenceFrame {
                position: std::array::from_fn(|i| new_frame[3][i]),
                basis: std::array::from_fn(|i| std::array::from_fn(|j| new_frame[i][j])),
            };
            publish_transform(
                pose,
                clear,
                &mut TriggerPublication {
                    reference: first,
                    world: &mut world,
                    callbacks: &mut callbacks,
                    properties: &properties,
                    commands: &commands,
                    cells: &cells,
                    grow_one_at_a_time: case & 1 != 0,
                },
            )
            .unwrap();
            commands
                .dispatch(&mut world, &cells, &mut callbacks)
                .unwrap();
        }
        let mut actual = Vec::new();
        for i in 1..=2 {
            let body = bodies.get_by_index(i).unwrap();
            actual.extend(body.transform.into_iter().flatten().map(f32::to_bits));
            actual.extend(body.bounds.words);
            actual.push(u32::from(body.status));
            actual.extend([
                u32::from(body.tim_angle),
                u32::from(body.max_tim_distance),
                u32::from(body.angular_tim_distance),
                u32::from(body.max_contact_distance),
                u32::from(body.lod_flags),
                u32::from(body.packed_average_surface_velocity),
            ]);
            actual.extend(body.motion_to_body_orientation.map(f32::to_bits));
        }
        for i in 1..=2 {
            let motion = motions.get(i).unwrap();
            actual.extend(
                motion
                    .pose
                    .center
                    .into_iter()
                    .chain(motion.pose.orientation)
                    .chain(motion.linear_velocity)
                    .chain(motion.angular_velocity)
                    .map(f32::to_bits),
            );
        }
        actual.push(broadphase.pending_body_bits().words()[0]);
        actual.extend(broadphase.pending_body_range().unwrap_or([u32::MAX, 0]));
        for i in 0..2 {
            actual.extend(waters.get(i).unwrap().plane().unwrap().map(f32::to_bits));
        }
        for i in 0..2 {
            let data = triggers.get(i).unwrap().velocities;
            actual.extend(
                data.scaled_linear
                    .into_iter()
                    .chain(data.scaled_angular)
                    .chain(data.requested_linear)
                    .chain(data.requested_angular)
                    .map(f32::to_bits),
            );
        }
        let trigger = triggers.get(0).unwrap();
        actual.extend([
            actors.bodies[trigger.movement_body].body_id,
            trigger.water.map_or(u32::MAX, |h| u32::from(h.slot)),
            u32::from(deferred.world_flags[0]),
            u32::from(deferred.world_flags[1]),
            u32::from(deferred.component_flag),
            trace.len() as u32,
        ]);
        actual.extend(trace);
        let expected: Vec<_> = words.collect();
        let diffs: Vec<_> = actual
            .iter()
            .zip(&expected)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .take(8)
            .map(|(i, (a, b))| format!("{i}:{a:08x}!={b:08x}"))
            .collect();
        assert_eq!(actual.len(), expected.len(), "length case{case}");
        assert!(diffs.is_empty(), "case{case} {}", diffs.join(","));
        cases += 1;
    }
    assert_eq!(cases, 1024);
}
