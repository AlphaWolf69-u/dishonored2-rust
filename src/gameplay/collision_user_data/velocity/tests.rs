use super::*;
struct World {
    case: u32,
    scale: f32,
    reference: u32,
    data: [CollisionVelocities; 3],
    events: Vec<u32>,
}
impl World {
    fn reference_changed(&mut self) {
        if self.case & 2 != 0 {
            self.reference = 0xab000002;
        }
    }
}
impl BodyVelocityWorld for World {
    type Reference = usize;
    fn body_id(&self, reference: usize) -> Result<u32, String> {
        assert_eq!(reference, 0);
        Ok(self.reference)
    }
    fn velocity_scale(&self, body: u32) -> Result<f32, String> {
        assert_eq!(body, 0xab000001);
        Ok(self.scale)
    }
    fn set_velocity(
        &mut self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String> {
        self.events.extend([2, body]);
        self.events
            .extend(linear.into_iter().chain(angular).map(f32::to_bits));
        self.reference_changed();
        Ok(())
    }
    fn queue_velocity(
        &mut self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String> {
        self.events.extend([3, 0x30, 0x90200, body]);
        self.events
            .extend(linear.into_iter().chain(angular).map(f32::to_bits));
        self.reference_changed();
        Ok(())
    }
    fn collision_velocities(&mut self, body: u32) -> Result<&mut CollisionVelocities, String> {
        Ok(&mut self.data[(body & 0xffffff) as usize - 1])
    }
    fn next_attached_body(&self, body: u32) -> Result<u32, String> {
        let slot = body & 0xffffff;
        Ok(if self.case & 4 != 0 && slot == 3 {
            0x00ffffff
        } else {
            0xab000000 + (slot % 3 + 1)
        })
    }
    fn body_capacity(&self) -> usize {
        4
    }
}

#[test]
fn collision_velocity_commands_match_executed_live_reference_and_cache_updates() {
    let mut cases = 0;
    for row in include_str!("../../../../tests/fixtures/collision_velocities.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let data = std::array::from_fn(|i| CollisionVelocities {
            scaled_linear: vector(10 + i * 16),
            scaled_angular: vector(14 + i * 16),
            requested_linear: vector(18 + i * 16),
            requested_angular: vector(22 + i * 16),
        });
        let mut world = World {
            case: w[0],
            scale: f32::from_bits(w[1]),
            reference: 0xab000001,
            data,
            events: Vec::new(),
        };
        publish_velocities(&mut world, 0, vector(2), vector(6), w[0] & 1 != 0).unwrap();
        let mut actual = Vec::new();
        for data in world.data {
            actual.extend(
                data.scaled_linear
                    .into_iter()
                    .chain(data.scaled_angular)
                    .chain(data.requested_linear)
                    .chain(data.requested_angular)
                    .map(f32::to_bits),
            );
        }
        actual.extend([world.reference, world.events.len() as u32]);
        actual.extend(world.events);
        assert_eq!(actual, w[58..], "case{}", w[0]);
        cases += 1;
    }
    assert_eq!(cases, 512);
}
