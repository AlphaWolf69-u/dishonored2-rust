use super::*;
fn snapshot(body: &CollisionBody) -> Vec<u32> {
    let mut result: Vec<_> = body
        .transform
        .into_iter()
        .flatten()
        .map(f32::to_bits)
        .collect();
    result.extend([
        body.motion_id,
        body.flags,
        u32::from(body.status),
        body.bounds.words[0],
        body.bounds.words[1],
        body.bounds.words[2],
        body.bounds.words[3],
        u32::from(body.lod_flags),
        u32::from(body.quality_id),
        u32::from(body.material_id),
        body.filter_info,
        body.id,
        body.next_attached_body_id,
        body.broadphase_id,
        u32::from(body.packed_radius),
        body.user_data as u32,
        (body.user_data >> 32) as u32,
        body.active_list_index,
    ]);
    result.extend(body.shape_selectors.map(u32::from));
    if let Some(slots) = body.shape_slots {
        result.push(8);
        result.extend(slots.map(|v| v as u32));
    } else {
        result.extend([1, body.shape as u32]);
    }
    result
}
#[test]
fn body_initialization_and_caches_match_executed_routines() {
    for (case, line) in include_str!("../../../tests/fixtures/collision_body.txt")
        .lines()
        .enumerate()
    {
        let mut w = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let id = w.next().unwrap();
        let flags = w.next().unwrap();
        let filter_info = w.next().unwrap();
        let material_id = w.next().unwrap() as u16;
        let quality_id = w.next().unwrap() as u8;
        let lo = w.next().unwrap();
        let hi = w.next().unwrap();
        let user_data = u64::from(lo) | (u64::from(hi) << 32);
        let collision_look_ahead_distance = f32::from_bits(w.next().unwrap());
        let position = std::array::from_fn(|_| f32::from_bits(w.next().unwrap()));
        let orientation = std::array::from_fn(|_| f32::from_bits(w.next().unwrap()));
        let old_w: [f32; 4] = std::array::from_fn(|_| f32::from_bits(w.next().unwrap()));
        let status = w.next().unwrap() as u16;
        let shape_flags = w.next().unwrap() as u16;
        let shape_type = w.next().unwrap() as u8;
        let selectors = std::array::from_fn(|_| w.next().unwrap() as u8);
        let shapes = std::array::from_fn(|_| w.next().unwrap() as usize);
        let mode = w.next().unwrap();
        let info = BodyInfo {
            shape: 0x123450,
            flags,
            filter_info,
            material_id,
            quality_id,
            user_data,
            position,
            orientation,
            collision_look_ahead_distance,
            ..Default::default()
        };
        let header = ShapeHeader {
            flags: shape_flags,
            shape_type,
            key_bits: 0,
            dispatch_type: 0,
            radius: 0.,
            user_data: 0,
        };
        let mut body = CollisionBody::default();
        body.status = status;
        for (column, old) in body.transform.iter_mut().zip(old_w) {
            column[3] = old;
        }
        body.initialize(id, &info, header, Some(ShapeSlots { shapes, selectors }))
            .unwrap();
        assert_eq!(body.shape, info.shape);
        let actual = snapshot(&body);
        let expected: Vec<_> = w.by_ref().take(actual.len()).collect();
        assert_eq!(actual, expected, "case {case} base body");
        for channel in 0..8 {
            assert_eq!(
                body.selected_shape(channel).unwrap(),
                if shape_type == 18 {
                    shapes[usize::from(selectors[channel] & 7)]
                } else {
                    info.shape
                }
            );
        }
        body.assign_motion_kind(mode != 0);
        let actual = snapshot(&body);
        let expected: Vec<_> = w.by_ref().take(actual.len()).collect();
        assert_eq!(actual, expected, "case {case} motion kind");
        body.refresh_inverse_frame();
        let actual: Vec<_> = body
            .transform
            .into_iter()
            .flatten()
            .chain(body.motion_to_body_orientation)
            .map(f32::to_bits)
            .collect();
        let expected: Vec<_> = w.by_ref().take(actual.len()).collect();
        assert_eq!(actual, expected, "case {case} inverse frame");
        body.lod_flags = w.next().unwrap() as u8;
        body.packed_radius = w.next().unwrap() as u16;
        let threshold = f32::from_bits(w.next().unwrap());
        let motion = w.next().unwrap() != 0;
        body.update_size_and_static_lod(threshold, motion);
        assert_eq!(
            u32::from(body.lod_flags),
            w.next().unwrap(),
            "case {case} quality flags"
        );
        assert_eq!(w.next(), None);
    }
}
