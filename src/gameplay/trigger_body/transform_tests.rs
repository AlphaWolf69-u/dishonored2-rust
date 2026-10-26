use super::*;
use crate::gameplay::{body_storage::BodyStorage, water_component::WaterComponent};
struct World<'a> {
    bodies: &'a BodyStorage,
    body: u32,
    water: Option<&'a mut WaterComponent>,
    mode: u32,
    reads: u32,
    height: f32,
    events: Vec<u32>,
}
impl World<'_> {
    fn frame(&mut self, frame: ShapeMatrix) {
        self.events
            .extend(frame.into_iter().flatten().map(f32::to_bits));
    }
    fn changed(&mut self) {
        if self.mode & 16 != 0 {
            self.body = 0xcd000002;
        }
        if self.mode & 64 != 0 {
            self.water = None;
        }
    }
}
impl TriggerTransformWorld for World<'_> {
    fn deferred(&mut self) -> bool {
        let value = (self.mode >> self.reads) & 1;
        self.reads += 1;
        self.events.extend([1, value]);
        value != 0
    }
    fn body_id(&self) -> Result<u32, String> {
        Ok(self.body)
    }
    fn set_transform(&mut self, body: u32, frame: ShapeMatrix) -> Result<(), String> {
        self.events.extend([2, body, 0]);
        self.frame(frame);
        self.changed();
        Ok(())
    }
    fn queue_transform(&mut self, body: u32, frame: ShapeMatrix) -> Result<(), String> {
        self.events.extend([3, 0x50, 0x60200, body, 0]);
        self.frame(frame);
        self.changed();
        Ok(())
    }
    fn reset_motion(&mut self, deferred: bool) -> Result<(), String> {
        self.events
            .extend([4, self.body, u32::from(deferred), 0, 0, 0, 0, 0, 0]);
        if self.mode & 32 != 0 {
            self.body = 0xab000001;
        }
        Ok(())
    }
    fn update_water_plane(&mut self, frame: ShapeMatrix) -> Result<(), String> {
        if let Some(water) = self.water.as_mut() {
            water.update_plane(|| {
                let shape = self
                    .bodies
                    .get_by_index(self.body & 0xffffff)
                    .ok_or("trigger transform body is missing")?
                    .shape;
                assert!(shape == 7 || shape == 9);
                self.events.push(if shape == 7 { 5 } else { 6 });
                self.events
                    .extend(frame.into_iter().flatten().map(f32::to_bits));
                let height = if shape == 9 {
                    self.height + 1.
                } else {
                    self.height
                };
                Ok([[0., 0., -4., 0.], [1., 1., height, 0.]])
            })?;
        }
        Ok(())
    }
}

#[test]
fn moving_water_notifications_match_complete_executed_trigger_transform() {
    let mut bodies = BodyStorage::new(3).unwrap();
    let id = bodies.reserve(&mut []).unwrap();
    bodies.get_mut(id).unwrap().shape = 7;
    let second = bodies.reserve(&mut []).unwrap();
    bodies.get_mut(second).unwrap().shape = 9;
    let mut cases = 0;
    for row in include_str!("../../../tests/fixtures/trigger_transform.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let case = w[0];
        let pose = ReferenceFrame {
            position: std::array::from_fn(|i| f32::from_bits(w[3 + i])),
            basis: std::array::from_fn(|i| {
                std::array::from_fn(|j| f32::from_bits(w[6 + i * 3 + j]))
            }),
        };
        let mut water = WaterComponent::default();
        water.owner.flags = 8;
        water
            .update_transform(bodies.get(id).unwrap(), [[0.; 4]; 4], |_, _| {
                Ok([[0.; 4], [0., 0., -5., 0.]])
            })
            .unwrap();
        water.owner.flags = w[1];
        let mut world = World {
            bodies: &bodies,
            body: 0xab000001,
            water: (case & 4 != 0).then_some(&mut water),
            mode: case,
            reads: 0,
            height: f32::from_bits(w[2]),
            events: Vec::new(),
        };
        publish_transform(pose, case & 8 != 0, &mut world).unwrap();
        let body = world.body;
        let events = std::mem::take(&mut world.events);
        drop(world);
        let count = w[15] as usize;
        assert_eq!(events, w[16..16 + count], "events case{case}");
        assert_eq!(
            water.plane().unwrap().map(f32::to_bits),
            w[16 + count..20 + count],
            "plane case{case}"
        );
        assert_eq!(body, w[20 + count], "live body case{case}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
