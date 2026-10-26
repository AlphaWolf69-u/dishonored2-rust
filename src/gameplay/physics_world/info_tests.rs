use super::*;

fn bytes(info: &WorldInfo) -> Vec<u8> {
    let mut out = vec![0; 0x110];
    let u32_at = |out: &mut [u8], at: usize, value: u32| {
        out[at..at + 4].copy_from_slice(&value.to_le_bytes())
    };
    for (at, value) in [
        (0, info.body_buffer_capacity),
        (0x10, info.motion_buffer_capacity),
        (0x20, info.constraint_buffer_capacity),
        (0x30, info.constraint_group_buffer_capacity),
        (0x64, info.num_splitter_cells),
        (0xfc, info.solver.iterations),
        (0x104, info.solver_microsteps),
    ] {
        u32_at(&mut out, at, value);
    }
    for (at, handle) in [
        (8, info.user_body_buffer),
        (0x18, info.user_motion_buffer),
        (0x28, info.user_constraint_buffer),
        (0x38, info.user_constraint_group_buffer),
        (0x40, info.persistent_stream_allocator),
        (0x48, info.material_library),
        (0x50, info.motion_properties_library),
        (0x58, info.quality_library),
        (0xb0, info.broad_phase_config),
        (0xb8, info.collision_filter),
        (0xc0, info.shape_tag_codec),
    ] {
        out[at..at + 8].copy_from_slice(&(handle.unwrap_or(0) as u64).to_le_bytes());
    }
    for (at, value) in [
        (0xc8, info.collision_tolerance),
        (0xcc, info.relative_collision_accuracy),
        (0xd0, info.aabb_margin),
        (0xf4, info.solver.tau),
        (0xf8, info.solver.damping),
        (0x108, info.max_approach_speed_for_high_quality_solver),
    ] {
        u32_at(&mut out, at, value.to_bits());
    }
    for (at, vector) in [
        (0x70, info.gravity),
        (0x90, info.broad_phase_aabb[0]),
        (0xa0, info.broad_phase_aabb[1]),
    ] {
        for (i, v) in vector.into_iter().enumerate() {
            u32_at(&mut out, at + i * 4, v.to_bits());
        }
    }
    out[0x60] = info.simulation_type;
    out[0x82] = info.broad_phase_type;
    for (at, flag) in [
        (0x80, info.enable_contact_caching),
        (0x81, info.merge_events_before_dispatch),
        (0xd4, info.enable_welding_for_default_objects),
        (0xd5, info.enable_welding_for_critical_objects),
        (0xf0, info.enable_sdf_edge_collisions),
        (0xf1, info.enable_collide_work_stealing),
        (0x100, info.solver_enable_post_step),
        (0x10c, info.enable_deactivation),
        (0x10d, info.enable_penetration_recovery),
        (0xd8, info.lod.register_default_config),
        (0xd9, info.lod.auto_build_on_dynamic_body_added),
        (0xda, info.lod.auto_build_on_mesh_body_added),
    ] {
        out[at] = u8::from(flag);
    }
    for (at, value) in [
        (0xdc, info.lod.accuracy),
        (0xe0, info.lod.slow_to_fast_threshold),
        (0xe4, info.lod.fast_to_slow_threshold),
        (0xe8, info.lod.body_is_big_threshold),
        (0xec, info.lod.average_velocity_gain),
    ] {
        u32_at(&mut out, at, value.to_bits());
    }
    out
}

#[test]
fn world_info_defaults_and_bounds_match_complete_executed_functions() {
    let mut constructors = 0;
    let mut bounds = 0;
    let mut arrays = 0;
    for row in include_str!("../../../tests/fixtures/world_info.txt").lines() {
        let mut words = row.split_whitespace();
        let kind = words.next().unwrap();
        let mut values = words.map(|v| u32::from_str_radix(v, 16).unwrap());
        let actual = match kind {
            "constructor" => {
                constructors += 1;
                bytes(&WorldInfo::default())
            }
            "array" => {
                arrays += 1;
                let count = values.next().unwrap() as usize;
                std::iter::repeat_with(WorldInfo::default)
                    .take(count)
                    .flat_map(|info| bytes(&info))
                    .collect()
            }
            "bounds" => {
                bounds += 1;
                let size = f32::from_bits(values.next().unwrap());
                let mut info = WorldInfo::default();
                info.set_broad_phase_size(size);
                info.broad_phase_aabb
                    .into_iter()
                    .flatten()
                    .flat_map(|v| v.to_bits().to_le_bytes())
                    .collect()
            }
            _ => panic!("unknown executed case"),
        };
        let expected: Vec<_> = values.flat_map(u32::to_le_bytes).collect();
        assert_eq!(actual, expected, "{kind} case {bounds}");
    }
    assert_eq!((constructors, arrays, bounds), (1, 4, 512));
}
