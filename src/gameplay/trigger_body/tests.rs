use super::*;
use crate::assets::physworld::body::BodyInfo;
fn body_words(body: &BodyInfo) -> Vec<u32> {
    let mut result = vec![
        body.shape as u32,
        body.flags,
        body.collision_control as u32,
        body.filter_info,
        body.material_id as u32,
        body.quality_id as u32,
        body.name as u32,
        body.user_data as u32,
        (body.user_data >> 32) as u32,
        body.motion_type as u32,
    ];
    result.extend(
        body.position
            .into_iter()
            .chain(body.orientation)
            .chain(body.linear_velocity)
            .chain(body.angular_velocity)
            .map(f32::to_bits),
    );
    result.extend([
        body.mass.to_bits(),
        body.mass_distribution as u32,
        body.motion_properties_id as u32,
        body.reserved_body_id,
        body.reserved_motion_id,
        body.collision_look_ahead_distance.to_bits(),
        body.local_frame as u32,
    ]);
    result
}

#[test]
pub(super) fn trigger_shape_requests_match_executed_component() {
    let decode = |value: &str| {
        if value == "-" {
            return String::new();
        }
        String::from_utf8(
            value
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect(),
        )
        .unwrap()
    };
    for (case, line) in include_str!("../../../tests/fixtures/trigger_shape_request.txt")
        .lines()
        .enumerate()
    {
        let mut fields = line.split_whitespace();
        let names: [String; 4] = std::array::from_fn(|_| decode(fields.next().unwrap()));
        let w: Vec<u32> = fields
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let flag = |i: u32| w[0] >> i & 1_u32 != 0;
        let request = trigger_shape_request(
            &names[0],
            &names[1],
            &names[2],
            std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[7 + i * 3 + j]))),
        );
        assert_eq!(request.name, names[3], "case {case}");
        let body = prepare_trigger_body(TriggerBodyInput {
            shape: 0x123450,
            position: std::array::from_fn(|i| f32::from_bits(w[4 + i])),
            basis: request.basis,
            material_id: w[1] as u16,
            user_data: 0x1234_5678_feed_face,
            water_component: flag(5),
            wind_component: flag(6),
            collision: TriggerCollisionSettings {
                debris: flag(0),
                rigid_bodies: flag(1),
                npc_ragdolls: flag(2),
                npc_capsules: flag(3),
                keyframed: flag(4),
            },
        });
        let start = if flag(4) { 20 } else { 21 };
        assert_eq!(body_words(&body), &w[start..], "case {case}");
    }
}
#[test]
pub(super) fn trigger_body_parameters_match_executed_component() {
    for (case, line) in include_str!("../../../tests/fixtures/trigger_body.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let flag = |i: u32| w[0] >> i & 1_u32 != 0;
        let body = prepare_trigger_body(TriggerBodyInput {
            shape: 0x123450,
            position: std::array::from_fn(|i| f32::from_bits(w[4 + i])),
            basis: std::array::from_fn(|i| {
                std::array::from_fn(|j| f32::from_bits(w[7 + i * 3 + j]))
            }),
            material_id: w[1] as u16,
            user_data: 0x1234_5678_feed_face,
            water_component: flag(5),
            wind_component: flag(6),
            collision: TriggerCollisionSettings {
                debris: flag(0),
                rigid_bodies: flag(1),
                npc_ragdolls: flag(2),
                npc_capsules: flag(3),
                keyframed: flag(4),
            },
        });
        let start = if flag(4) { 20 } else { 21 };
        assert_eq!(body_words(&body), &w[start..], "case {case}");
    }
}
