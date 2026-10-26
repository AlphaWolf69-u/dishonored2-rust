use super::*;
use crate::gameplay::{
    body_activation::ActivationOwners,
    body_quality::{BodyQualityConfig, BodyQualityLibrary},
    body_storage::BodyStorage,
    broadphase::BroadphaseQuantization,
    broadphase::HybridBroadphase,
    constraint_links::ConstraintLinks,
    deactivation::DeactivationWorld,
    motion_info::MotionInfo,
    motion_storage::MotionStorage,
    physics_world::{
        PublicationContext, PublicationEvents, PublicationGeometry, TransformCallbacks,
    },
    space_cells::SpaceCells,
};

struct Geometry<'a> {
    has_shape: bool,
    local: [[f32; 4]; 2],
    events: &'a mut Vec<u32>,
}
impl PublicationGeometry for Geometry<'_> {
    fn is_null(&mut self, shape: usize) -> Result<bool, String> {
        assert_eq!(shape, 7);
        Ok(!self.has_shape)
    }
    fn bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        assert_eq!(shape, 7);
        assert!(self.has_shape);
        self.events
            .extend(frame.into_iter().flatten().map(f32::to_bits));
        Ok(crate::gameplay::shape_transform::transform_bounds(
            frame, self.local,
        ))
    }
}
struct NoEvents;
impl PublicationEvents for NoEvents {
    fn activation(&mut self, _: ActivationOwners<'_>, _: &[u32], _: bool) -> Result<(), String> {
        panic!("cache refresh must not emit activation")
    }
    fn added(&mut self, _: ActivationOwners<'_>, _: &[u32]) -> Result<(), String> {
        panic!("cache refresh must not emit addition")
    }
    fn transform_changed(
        &mut self,
        _: ActivationOwners<'_>,
        _: u32,
        _: bool,
    ) -> Result<(), String> {
        panic!("cache refresh must not emit a transform signal")
    }
}
fn vector(words: &mut impl Iterator<Item = u32>) -> [f32; 4] {
    std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
}

#[test]
fn motion_cache_matches_complete_executed_body_frame_bounds_and_lod() {
    let mut cases = 0;
    let mut errors = Vec::new();
    for row in include_str!("../../../../tests/fixtures/body_motion_cache.txt").lines() {
        let mut w = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let case = w.next().unwrap();
        let qbits = w.next().unwrap();
        let qid = w.next().unwrap() as u8;
        let has_shape = w.next().unwrap() != 0;
        let has_lod = w.next().unwrap() != 0;
        let mut body = CollisionBody::default();
        body.transform = std::array::from_fn(|_| vector(&mut w));
        body.motion_to_body_orientation = vector(&mut w);
        body.status = w.next().unwrap() as u16;
        body.lod_flags = w.next().unwrap() as u8;
        body.packed_average_surface_velocity = w.next().unwrap() as u16;
        body.packed_radius = w.next().unwrap() as u16;
        body.tim_angle = w.next().unwrap() as u16;
        body.max_tim_distance = w.next().unwrap() as u16;
        body.angular_tim_distance = w.next().unwrap() as u16;
        body.max_contact_distance = w.next().unwrap() as u16;
        body.bounds = PackedBodyBounds {
            words: std::array::from_fn(|_| w.next().unwrap()),
        };
        body.quality_id = qid;
        let mut motion = Motion::default();
        motion.pose.center = vector(&mut w);
        motion.pose.orientation = vector(&mut w);
        motion.linear_velocity = vector(&mut w);
        motion.angular_velocity = vector(&mut w);
        motion.previous_step_linear_velocity = vector(&mut w);
        motion.previous_step_angular_velocity = vector(&mut w);
        motion.linear_velocity_cage = std::array::from_fn(|_| w.next().unwrap() as u16);
        motion.integration_factor = w.next().unwrap() as u16;
        let tolerance = vector(&mut w);
        let dt = vector(&mut w);
        let rotation = f32::from_bits(w.next().unwrap());
        let contact = vector(&mut w);
        let margin = f32::from_bits(w.next().unwrap());
        let lod = BodyLodConfig {
            slow_to_fast_threshold: f32::from_bits(w.next().unwrap()),
            fast_to_slow_threshold: f32::from_bits(w.next().unwrap()),
            average_velocity_gain: f32::from_bits(w.next().unwrap()),
            ..Default::default()
        };
        let local = [vector(&mut w), vector(&mut w)];
        let world = [vector(&mut w), vector(&mut w)];
        let packing = BroadphaseQuantization::new(world);
        let step = SweptBoundsStep {
            duration: dt,
            rotation_test_scale: rotation,
            contact_distance_scale: contact,
        };
        let settings = BodyCacheSettings {
            bounds_tolerance: tolerance,
            cache_margin: margin,
            lod: has_lod.then_some(lod),
            step: &step,
            packing: &packing.packed,
        };
        let qualities = BodyQualityLibrary::new(BodyQualityConfig {
            enable_welding_for_default_objects: qbits & 1 != 0,
            enable_welding_for_critical_objects: qbits & 2 != 0,
            enable_contact_caching: qbits & 4 != 0,
        });
        let mut events = Vec::new();
        let mut bodies = BodyStorage::new(2).unwrap();
        let id = bodies.reserve(&mut []).unwrap();
        let mut motions = MotionStorage::new(2).unwrap();
        let motion_id = motions.reserve().unwrap();
        motions
            .initialize(motion_id, &MotionInfo::default(), &SpaceCells::Single)
            .unwrap();
        motion.space_splitter_weight = 1; // The shared owner retains only live motions.
        *motions.get_mut(motion_id).unwrap() = motion;
        body.id = id;
        body.motion_id = motion_id;
        body.shape = 9; // The selected channel must not use the main resource.
        body.shape_selectors = [0x83; 8];
        body.shape_slots = Some([7; 8]);
        *bodies.get_mut(id).unwrap() = body;
        let mut deactivation = DeactivationWorld::new(Vec::new());
        let mut broadphase = HybridBroadphase::with_body_capacity(2);
        let mut geometry = Geometry {
            has_shape,
            local,
            events: &mut events,
        };
        let mut observers = NoEvents;
        PublicationContext {
            geometry: &mut geometry,
            events: &mut observers,
            cache: settings,
            qualities: &qualities,
            constraints: ConstraintLinks {
                heads: &[],
                constraints: &[],
                next: &[],
            },
            enable_deactivation: false,
            phase: 0,
            step_index: 0,
        }
        .refresh_transform_cache(
            ActivationOwners {
                bodies: &mut bodies,
                motions: &mut motions,
                deactivation: &mut deactivation,
                broadphase: &mut broadphase,
            },
            id,
        )
        .unwrap();
        let body = bodies.get(id).unwrap();
        let mut actual: Vec<_> = body
            .transform
            .into_iter()
            .flatten()
            .map(f32::to_bits)
            .collect();
        actual.extend([
            u32::from(body.tim_angle),
            u32::from(body.max_tim_distance),
            u32::from(body.angular_tim_distance),
            u32::from(body.max_contact_distance),
            u32::from(body.status),
            u32::from(body.lod_flags),
            u32::from(body.packed_average_surface_velocity),
        ]);
        actual.extend(body.bounds.words);
        actual.push(events.len() as u32);
        actual.extend(events);
        let expected = w.collect::<Vec<_>>();
        if actual != expected {
            let differences: Vec<_> = actual
                .iter()
                .zip(&expected)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .take(8)
                .map(|(i, (a, b))| format!("{i}:{a:08x}!={b:08x}"))
                .collect();
            errors.push(format!("case{case} {}", differences.join(",")));
        }
        cases += 1;
    }
    assert_eq!(cases, 1024);
    assert!(
        errors.is_empty(),
        "{} cases differ: {}",
        errors.len(),
        errors
            .iter()
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
}
