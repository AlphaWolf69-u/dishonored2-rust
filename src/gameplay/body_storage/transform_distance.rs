//! c88440 accumulates movement over a body's live attachment ring.
use super::BodyStorage;
use crate::float::{add, multiply, truncate_i32};

fn distance(value: f32) -> u16 {
    // MAXPS(value,0), MINPS(value,65535), CVTTPS2DQ, then low16.
    let value = if value > 0. { value } else { 0. };
    let value = if value < 65535. { value } else { 65535. };
    truncate_i32(value) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distance_accumulation_matches_executed_live_attachment_rings() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/transform_distance.txt").lines() {
            let w: Vec<_> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let mut bodies = BodyStorage::new(4).unwrap();
            for slot in 1..4 {
                let at = 13 + (slot - 1) * 7;
                let body = &mut bodies.bodies[slot];
                body.id = w[at];
                body.next_attached_body_id = w[at + 1];
                body.status = w[at + 2] as u16;
                body.tim_angle = w[at + 3] as u16;
                body.max_tim_distance = w[at + 4] as u16;
                body.angular_tim_distance = w[at + 5] as u16;
                body.packed_radius = w[at + 6] as u16;
            }
            let mut events = Vec::new();
            bodies
                .accumulate_transform_distance(
                    0xab000001,
                    vector(1),
                    vector(5),
                    vector(9),
                    |bodies, id| {
                        events.push(id);
                        let body = &mut bodies.bodies[(id & 0xffffff) as usize];
                        if w[0] & 8 != 0 {
                            body.max_tim_distance ^= 0xa5a5;
                            body.angular_tim_distance ^= 0x5a5a;
                            body.packed_radius = 0x4000;
                        }
                        if w[0] & 16 != 0 && id & 0xffffff == 1 {
                            body.next_attached_body_id = 0xab000001;
                        }
                        Ok(())
                    },
                )
                .unwrap();
            let mut actual = Vec::new();
            for body in &bodies.bodies[1..4] {
                actual.extend([
                    body.id,
                    body.next_attached_body_id,
                    u32::from(body.status),
                    u32::from(body.tim_angle),
                    u32::from(body.max_tim_distance),
                    u32::from(body.angular_tim_distance),
                    u32::from(body.packed_radius),
                ]);
            }
            actual.push(events.len() as u32);
            actual.extend(events);
            assert_eq!(actual, w[34..], "case{}", w[0]);
            count += 1;
        }
        assert_eq!(count, 512);
    }
}

impl BodyStorage {
    /// Refresh12816e0 is a required world operation and may change each body
    /// before its counters and next link are read. Distances use lane0, as the
    /// source's packed counters do; scale comes from world+730.
    pub fn accumulate_transform_distance(
        &mut self,
        first: u32,
        linear: [f32; 4],
        angular: [f32; 4],
        scale: [f32; 4],
        mut refresh: impl FnMut(&mut Self, u32) -> Result<(), String>,
    ) -> Result<(), String> {
        let linear = distance(multiply(linear[0], scale[0]));
        let angle = truncate_i32(add(
            multiply(angular[0], f32::from_bits(0x4722_f8e0)),
            f32::from_bits(0x3f7f_f972),
        ))
        .clamp(0, 65535) as u16;
        let mut current = first;
        let mut visited = 0;
        loop {
            if visited >= self.bodies.len() {
                return Err(
                    "transform-distance attachment ring does not return to its first body".into(),
                );
            }
            visited += 1;
            refresh(self, current)?;
            let body = self
                .bodies
                .get_mut((current & 0xffffff) as usize)
                .ok_or("transform-distance body index exceeds storage")?;
            let radius = f32::from_bits(u32::from(body.packed_radius) << 16);
            let rotation = distance(multiply(multiply(radius, angular[0]), scale[0]));
            body.max_tim_distance =
                (u32::from(body.max_tim_distance) + u32::from(linear) + u32::from(rotation))
                    .min(65535) as u16;
            body.angular_tim_distance = body.angular_tim_distance.saturating_add(rotation);
            body.tim_angle = body.tim_angle.saturating_add(angle);
            body.status |= 4;
            current = body.next_attached_body_id;
            if current == first {
                return Ok(());
            }
        }
    }
}
