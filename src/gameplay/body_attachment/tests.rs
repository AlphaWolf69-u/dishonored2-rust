use super::*;
use crate::gameplay::{
    body_quality::BodyQualityConfig, motion_info::MotionInfo, space_cells::SpaceCells,
};
use std::cell::RefCell;

struct Inputs<'a>(std::str::SplitWhitespace<'a>);
impl Inputs<'_> {
    fn word(&mut self) -> u32 {
        u32::from_str_radix(self.0.next().unwrap(), 16).unwrap()
    }
    fn words<const N: usize>(&mut self) -> [u32; N] {
        std::array::from_fn(|_| self.word())
    }
    fn vector(&mut self) -> [f32; 4] {
        self.words::<4>().map(f32::from_bits)
    }
}
struct Queries {
    flags: u16,
    measure: f32,
    local: [[f32; 4]; 2],
    events: RefCell<Vec<u32>>,
}
impl AttachmentWorld for Queries {
    fn shape_flags(&mut self, shape: usize) -> Result<u16, String> {
        assert_eq!(shape, 1);
        Ok(self.flags)
    }
    fn rotation_limit_measure(&mut self, shape: usize) -> Result<f32, String> {
        assert_eq!(shape, 1);
        self.events.borrow_mut().push(2);
        Ok(self.measure)
    }
    fn shape_bounds(&mut self, shape: usize, frame: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        assert_eq!(shape, 1);
        self.events.borrow_mut().push(3);
        Ok(crate::gameplay::shape_transform::transform_bounds(frame, self.local))
    }
    fn register_deactivation_body(&mut self, body: u32, motion: u32) -> Result<(), String> {
        assert_eq!((body, motion), (0x01000001, 1));
        self.events.borrow_mut().push(1);
        Ok(())
    }
}

#[test]
fn attachments_match_executed_complete_routine() {
    let mut count = 0;
    for (case, line) in include_str!("../../../tests/fixtures/body_attachment.txt")
        .lines()
        .enumerate()
    {
        let mut input = Inputs(line.split_whitespace());
        let config = input.word();
        let quality_id = input.word() as u8;
        let flags = input.word() as u16;
        let measure = f32::from_bits(input.word());
        let solver_id = input.word();
        let inverse_mass = input.words::<4>().map(|v| v as u16);
        let limits = input.words::<2>().map(|v| v as u16);
        let transform = std::array::from_fn(|_| input.vector());
        let center = input.vector();
        let orientation = input.vector();
        let linear = input.vector();
        let angular = input.vector();
        let cage = input.words::<4>().map(|v| v as u16);
        let broadphase = input.word();
        let body_flags = input.word();
        let status = input.word() as u16;
        let lod_flags = input.word() as u8;
        let reference = input.word() as u16;
        let radius = input.word() as u16;
        let contact = input.word() as u16;
        let tim = input.word() as u16;
        let packed = input.words::<4>();
        let local = [input.vector(), input.vector()];
        let world_bounds = [input.vector(), input.vector()];
        let tolerance = input.vector();
        let duration = input.vector();
        let rotation_scale = f32::from_bits(input.word());
        let contact_scale = input.vector();
        let thresholds = input.words::<3>().map(f32::from_bits);
        let supplied = input.word() != 0;
        let mut bodies = BodyStorage::new(2).unwrap();
        let body_id = bodies.reserve(&mut []).unwrap();
        let body = bodies.get_mut(body_id).unwrap();
        body.shape = 1;
        body.transform = transform;
        body.quality_id = quality_id;
        body.broadphase_id = broadphase;
        body.flags = body_flags;
        body.status = status;
        body.lod_flags = lod_flags;
        body.packed_average_surface_velocity = reference;
        body.packed_radius = radius;
        body.max_contact_distance = contact;
        body.max_tim_distance = tim;
        body.bounds.words = packed;
        let mut motions = MotionStorage::new(2).unwrap();
        let motion_id = motions.reserve().unwrap();
        motions
            .initialize(motion_id, &MotionInfo::default(), &SpaceCells::Single)
            .unwrap();
        let motion = motions.get_mut(motion_id).unwrap();
        motion.solver_id = solver_id;
        motion.inverse_mass = inverse_mass;
        motion.limits = limits;
        motion.pose.center = center;
        motion.pose.orientation = orientation;
        motion.linear_velocity = linear;
        motion.angular_velocity = angular;
        motion.linear_velocity_cage = [cage[0], cage[1], cage[2]];
        motion.integration_factor = cage[3];
        let qualities = BodyQualityLibrary::new(BodyQualityConfig {
            enable_welding_for_default_objects: config & 1 != 0,
            enable_welding_for_critical_objects: config & 2 != 0,
            enable_contact_caching: config & 4 != 0,
        });
        let lod = BodyLodConfig {
            slow_to_fast_threshold: thresholds[0],
            fast_to_slow_threshold: thresholds[1],
            body_is_big_threshold: thresholds[2],
            ..Default::default()
        };
        let packing = crate::gameplay::broadphase::BroadphaseQuantization::new(world_bounds);
        let step = SweptBoundsStep {
            duration,
            rotation_test_scale: rotation_scale,
            contact_distance_scale: contact_scale,
        };
        let settings = AttachmentSettings {
            qualities: &qualities,
            lod: &lod,
            tolerance,
            step: &step,
            packing: &packing.packed,
        };
        let mut world = Queries {
            flags,
            measure,
            local,
            events: RefCell::new(Vec::new()),
        };
        attach_body_motion(
            &mut bodies,
            &mut motions,
            body_id,
            motion_id,
            supplied.then_some([0., 0., 0., 1.]),
            &settings,
            &mut world,
        )
        .unwrap();
        let body = bodies.get(body_id).unwrap();
        let motion = motions.get(motion_id).unwrap();
        let mut result = vec![
            motion.first_body_id,
            u32::from(motion.limits[0]),
            u32::from(motion.limits[1]),
        ];
        result.extend(
            body.transform
                .into_iter()
                .flatten()
                .chain(body.motion_to_body_orientation)
                .map(f32::to_bits),
        );
        result.extend([
            body.motion_id,
            body.flags,
            u32::from(body.status),
            u32::from(body.max_tim_distance),
            u32::from(body.lod_flags),
            body.next_attached_body_id,
            u32::from(body.max_contact_distance),
            u32::from(body.packed_radius),
            u32::from(body.packed_average_surface_velocity),
        ]);
        result.extend(body.bounds.words);
        let expected: Vec<u32> = (0..result.len()).map(|_| input.word()).collect();
        assert_eq!(result, expected, "case {case}");
        let n = input.word() as usize;
        let events: Vec<_> = (0..n).map(|_| input.word()).collect();
        assert_eq!(
            *world.events.borrow(),
            events,
            "case {case} world operation order"
        );
        assert!(input.0.next().is_none());
        count += 1;
    }
    assert_eq!(count, 1024);
}
