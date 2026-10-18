use super::*;

#[test]
fn entity_clock_reset_and_getter_match_original_code() {
    let mut count = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/model_clock_cases.txt"
    ))
    .lines()
    {
        let (operation, values) = line.split_once(' ').unwrap();
        let words: Vec<u64> = values
            .split_whitespace()
            .map(|word| u64::from_str_radix(word, 16).unwrap())
            .collect();
        match operation {
            "reset" => {
                let mut group = words[3] as u32;
                let mut offset = f32::from_bits(words[4] as u32);
                let mut flags = words[1] as u8;
                let model = (words[0] != 0).then_some((&mut offset, &mut flags));
                reset_entity_shader_clock(&mut group, f64::from_bits(words[2]), model);
                assert_eq!(
                    [
                        u64::from(group),
                        u64::from(offset.to_bits()),
                        u64::from(flags)
                    ],
                    [words[5], words[6], words[7]],
                    "{line}"
                );
                if words[0] != 0 {
                    assert_eq!(model_shader_time_group(flags), 3);
                }
            }
            "get" => {
                let mut records = [[0xa5; 0x50]; 5];
                records[words[0] as usize][0x18..0x20].copy_from_slice(&words[1].to_le_bytes());
                assert_eq!(
                    game_clock_group_time(&records, words[0] as usize)
                        .unwrap()
                        .to_bits(),
                    words[2]
                );
                assert!(game_clock_group_time(&records, records.len()).is_none());
            }
            _ => panic!("unknown native clock case {line}"),
        }
        count += 1;
    }
    assert_eq!(count, 9306);
}

#[test]
fn model_shader_time_preserves_native_group_and_double_addition() {
    for flags in 0..=u8::MAX {
        assert_eq!(
            model_shader_time_group(flags),
            usize::from((flags & 0x70) >> 4)
        );
    }
    let time = model_shader_local_time(0.5, 16_777_216.75, -0.0);
    assert_eq!(time[0], 16_777_218.0);
    assert_ne!(
        time[0],
        16_777_216.75f64 as f32 + 0.5,
        "rounding renderer time before adding the model offset loses the native result"
    );
    assert_eq!(time[1].to_bits(), (-0.0f32).to_bits());
    assert_eq!(model_shader_local_time(-1.0, 0.25, 0.125), [-0.75, 0.125]);
}

#[test]
fn shadow_state_uses_published_mvp_winding_not_renderer_enablement() {
    let mut programs = [0; 22];
    programs[10] = 0x1234;
    let surface = DepthAlphaSurface {
        flags49: 0,
        default_programs: &programs,
        override_programs: &[],
        selector: -1,
        has_optional_mesh: false,
        special_vertex_format: false,
        alternate_program: 0,
        draw_state: [0; 9],
    };
    let mut mvp = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    for (scale, expected) in [(1.0, 1), (-1.0, 0)] {
        mvp[0][0] = scale;
        let plan = prepare_shadow_model_draw_for_mvp(&surface, [0.0; 3], 0.0, &mvp)
            .unwrap()
            .unwrap();
        assert_eq!(plan.program, programs[10]);
        assert_eq!(plan.draw_state[5] >> 27 & 1, expected);
    }
}

#[test]
fn depth_owner_parent_and_local_refresh_keep_native_rules() {
    let root = parent_model_depth_state(None);
    assert_eq!(
        (root.dispatch_flags, root.model_mask, root.depth_hack),
        (0, 1, 0.0)
    );
    let missing_model = parent_model_depth_state(Some((0xfe, None)));
    assert_eq!(
        (
            missing_model.dispatch_flags,
            missing_model.model_mask,
            missing_model.depth_hack
        ),
        (1, 1, 0.0)
    );
    for flags in 0..=255 {
        let parent = parent_model_depth_state(Some((flags, Some((flags, -0.125)))));
        assert_eq!(parent.dispatch_flags, (flags >> 1) & 1);
        assert_eq!(parent.model_mask, (flags >> 3) & 1);
        assert_eq!(parent.depth_hack, -0.125);
    }
    let mut local = 0.0;
    assert!(!set_local_model_depth_hack(&mut local, -0.0));
    assert_eq!(local.to_bits(), 0.0f32.to_bits());
    assert!(set_local_model_depth_hack(&mut local, 0.5));
    assert!(!set_local_model_depth_hack(&mut local, 0.5));
    assert!(set_local_model_depth_hack(&mut local, f32::NAN));
    assert!(set_local_model_depth_hack(&mut local, f32::NAN));
}

#[test]
fn depth_owner_matches_original_three_node_trees() {
    let mut count = 0;
    for (row, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/model_depth_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let words: Vec<u32> = line
            .split_whitespace()
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect();
        assert_eq!(words.len(), 36);
        let mut inherited = InheritedModelDepthState {
            dispatch_flags: words[0] as u8,
            model_mask: words[1] as u8,
            depth_hack: f32::from_bits(words[2]),
        };
        for node in 0..3 {
            let input = &words[3 + node * 5..3 + (node + 1) * 5];
            let expected = &words[18 + node * 6..18 + (node + 1) * 6];
            inherited = resolve_inherited_model_depth_state(
                inherited,
                f32::from_bits(input[0]),
                input[1] as u8,
                input[2] as u8,
            );
            let mut flags = input[4] as u8;
            let mut published = f32::from_bits(0x42c8_0000);
            let present = input[3] != 0;
            let override_value = if present {
                publish_model_depth_state(&mut flags, &mut published, inherited)
            } else {
                None
            };
            let actual = [
                u32::from(inherited.dispatch_flags != 0),
                u32::from(present),
                u32::from(override_value.is_some()),
                override_value.map_or(0, f32::to_bits),
                published.to_bits(),
                u32::from(flags),
            ];
            assert_eq!(
                actual.as_slice(),
                expected,
                "original depth owner row {row} node {node}"
            );
            count += 1;
        }
    }
    assert_eq!(count, 768);
}

#[test]
fn depth_owner_inheritance_preserves_original_nonzero_and_unit_rules() {
    for parent in [0.0, -0.0, 1.0, 0.125, -2.0, f32::NAN] {
        for local in [0.0, -0.0, 1.0, 0.25, -3.0, f32::NAN] {
            for inherited_mask in [0, 1, 0xff] {
                for actor_mask in [0, 1, 0xfe, 0xff] {
                    let resolved = resolve_inherited_model_depth_state(
                        InheritedModelDepthState {
                            dispatch_flags: 0x80,
                            model_mask: inherited_mask,
                            depth_hack: parent,
                        },
                        local,
                        0x81,
                        actor_mask,
                    );
                    let depth = if local != 0.0 { local } else { parent };
                    assert_eq!(resolved.depth_hack.to_bits(), depth.to_bits());
                    assert_eq!(resolved.dispatch_flags, 0x81);
                    let expected_mask = if depth == 0.0 {
                        actor_mask & 1
                    } else {
                        actor_mask & inherited_mask & 1
                    };
                    assert_eq!(resolved.model_mask, expected_mask);
                    let mut flags = 0xa5;
                    let mut published = -100.0;
                    let override_value =
                        publish_model_depth_state(&mut flags, &mut published, resolved);
                    assert_eq!(flags, (0xa5 & !8) | (expected_mask << 3));
                    assert_eq!(published.to_bits(), depth.to_bits());
                    assert_eq!(
                        override_value.map(f32::to_bits),
                        (depth > 0.0 && depth != 1.0).then_some(depth.to_bits())
                    );
                }
            }
        }
    }
}

#[test]
fn shadow_source_resolves_bank_not_capture_identifier() {
    let opaque = 2 | 0x800;
    let blend = (4 << 1) | (5 << 5) | 0x800;
    use ShadowProgramSource::*;
    assert_eq!(
        shadow_program_source(false, false, false, false, false, opaque, false),
        MainProgram
    );
    assert_eq!(
        shadow_program_source(false, false, false, false, false, blend, false),
        Disabled
    );
    assert_eq!(
        shadow_program_source(false, false, false, false, true, blend, false),
        MainProgram
    );
    assert_eq!(
        shadow_program_source(false, false, false, true, false, blend, false),
        Authored
    );
    assert_eq!(
        shadow_program_source(false, true, false, true, false, opaque, false),
        Disabled
    );
    assert_eq!(
        shadow_program_source(true, true, false, true, false, opaque, false),
        Authored
    );
    assert_eq!(
        shadow_program_source(true, false, false, false, false, opaque, false),
        Disabled
    );
    assert_eq!(
        shadow_program_source(true, false, false, false, false, opaque, true),
        MainProgram
    );
    assert_eq!(
        shadow_program_source(true, false, true, true, false, opaque, true),
        Disabled
    );
}

#[test]
fn native_blend_enable_is_independent_of_sort_and_blend_operation() {
    for source in 0..10u32 {
        for destination in 0..10u32 {
            for alpha_source in 0..10u32 {
                for alpha_destination in 0..10u32 {
                    let word = (source << 1)
                        | (destination << 5)
                        | (alpha_source << 11)
                        | (alpha_destination << 15);
                    let expected = source != 1
                        || destination != 0
                        || alpha_source != 1
                        || alpha_destination != 0;
                    assert_eq!(native_blend_enabled(word), expected);
                    assert_eq!(
                        native_blend_enabled(word | 0xffff_f601 & !0x7ffff),
                        expected
                    );
                    assert_eq!(native_blend_enabled(word | 0x180601), expected);
                }
            }
        }
    }
}

#[test]
fn shadow_model_draw_matches_original_program_state_branches() {
    // Original 329120 program/state instructions execute unchanged;
    // bounds/matrix and external renderer callees are intercepted.
    let default: Vec<u64> = (100..144).collect();
    let overrides: Vec<u64> = (1000..1330).collect();
    let mut count = 0;
    for (case, line) in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/shadow_model_draw_cases.txt"
    ))
    .lines()
    .enumerate()
    {
        let words: Vec<i64> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(words.len(), 29);
        let surface = DepthAlphaSurface {
            flags49: words[0] as u8,
            selector: words[1] as i8,
            has_optional_mesh: words[2] != 0,
            special_vertex_format: words[3] != 0,
            alternate_program: 0,
            default_programs: &default,
            override_programs: &overrides,
            draw_state: std::array::from_fn(|i| words[9 + i] as u32),
        };
        let actual = prepare_shadow_model_draw(
            &surface,
            std::array::from_fn(|i| f32::from_bits(words[6 + i] as u32)),
            f32::from_bits(words[4] as u32),
            words[5] != 0,
        )
        .unwrap();
        assert_eq!(actual.is_some(), words[18] != 0, "row {case}");
        if let Some(actual) = actual {
            assert_eq!(actual.program, words[19] as u64, "program row {case}");
            assert_eq!(
                actual.draw_state,
                std::array::from_fn(|i| words[20 + i] as u32),
                "state row {case}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 5760);
    let surface = DepthAlphaSurface {
        flags49: 0,
        selector: -1,
        has_optional_mesh: false,
        special_vertex_format: false,
        alternate_program: 1,
        default_programs: &[0; 44],
        override_programs: &[],
        draw_state: [u32::MAX; 9],
    };
    assert!(prepare_shadow_model_draw(&surface, [0.0; 3], 0.0, true)
        .unwrap()
        .is_none());
}

#[test]
fn depth_alpha_draw_matches_original_three_pass_dispatch() {
    // probe_original_depth_alpha_draw.py executes unmodified 321860
    // list/program/state branches with rendering/TLS callees intercepted.
    let default: Vec<u64> = (100..144).collect();
    let overrides: Vec<u64> = (1000..1330).collect();
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/depth_alpha_draw_cases.txt"
    ));
    let mut count = 0;
    for (index, line) in fixture.lines().enumerate() {
        let words: Vec<i64> = line
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(words.len(), 26);
        let surface = DepthAlphaSurface {
            flags49: words[0] as u8,
            selector: words[1] as i8,
            has_optional_mesh: words[2] != 0,
            special_vertex_format: words[3] != 0,
            alternate_program: 77,
            default_programs: &default,
            override_programs: &overrides,
            draw_state: words[5..14]
                .iter()
                .map(|v| *v as u32)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        };
        let actual = prepare_depth_alpha_draw(&surface, words[4] as u8).unwrap();
        assert_eq!(actual.is_some(), words[14] != 0, "native row{index}");
        if let Some(actual) = actual {
            assert_eq!(
                actual.program, words[15] as u64,
                "native program row{index}"
            );
            assert_eq!(
                actual.stencil_reference, words[16] as u8,
                "native stencil row{index}"
            );
            assert_eq!(
                actual.draw_state.as_slice(),
                words[17..26].iter().map(|v| *v as u32).collect::<Vec<_>>(),
                "native state row{index}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 1440);
}

#[test]
fn depth_alpha_draw_selection_and_native_state_masks() {
    let default: Vec<u64> = (100..144).collect();
    let overrides: Vec<u64> = (1000..1330).collect();
    let mut surface = DepthAlphaSurface {
        flags49: 0,
        default_programs: &default,
        override_programs: &overrides,
        selector: -1,
        has_optional_mesh: false,
        special_vertex_format: false,
        alternate_program: 77,
        draw_state: [u32::MAX; 9],
    };
    for flags in [0, 1, 0x40, 0x41] {
        for optional in [false, true] {
            for special in [false, true] {
                surface.flags49 = flags;
                surface.has_optional_mesh = optional;
                surface.special_vertex_format = special;
                for selector in [-1, 0, 1, 2, 3, 6, 7, -128] {
                    surface.selector = selector;
                    for pass in 0..3 {
                        let plan = prepare_depth_alpha_draw(&surface, pass).unwrap();
                        if pass == 2 && flags & 1 == 0 {
                            assert!(plan.is_none());
                            continue;
                        }
                        let plan = plan.unwrap();
                        let slot = match (pass, flags & 1) {
                            (0, 1) => 4,
                            _ => 6,
                        } + u64::from(special);
                        let expected_program = if pass == 2 {
                            77
                        } else if selector != -1 && flags & 0x40 != 0 {
                            let row = match selector {
                                0 => 0,
                                1 => 1,
                                2 => 2,
                                3..=6 => 3,
                                _ => 4,
                            };
                            1000 + (row * 3 + u64::from(optional)) * 22 + slot
                        } else {
                            100 + u64::from(optional) * 22 + slot
                        };
                        assert_eq!(plan.program, expected_program);
                        assert_eq!(plan.stencil_reference, if pass == 2 { 0x80 } else { 2 });
                        assert_eq!(plan.marks_alpha, pass == 0 && flags & 1 != 0);
                        assert_eq!(
                            plan.draw_state,
                            [
                                if plan.marks_alpha {
                                    0xfe5f_fe23
                                } else {
                                    0xfe1f_ffff
                                },
                                0xfe1f_ffff,
                                u32::MAX,
                                u32::MAX,
                                0xe80e_80ff,
                                if pass == 2 { 0xff80_80ef } else { 0xff02_02ff },
                                u32::MAX,
                                u32::MAX,
                                u32::MAX
                            ]
                        );
                    }
                }
            }
        }
    }
    surface.flags49 = 1;
    surface.alternate_program = 0;
    assert!(prepare_depth_alpha_draw(&surface, 2).unwrap().is_none());
    surface.flags49 = 0;
    surface.selector = -1;
    surface.has_optional_mesh = false;
    surface.default_programs = &[0; 44];
    assert!(prepare_depth_alpha_draw(&surface, 0).unwrap().is_none());
    assert!(prepare_depth_alpha_draw(&surface, 3).is_err());
    surface.default_programs = &[];
    assert!(prepare_depth_alpha_draw(&surface, 0).is_err());
}

#[test]
fn native_multisample_getters_match_original() {
    assert_eq!(
        original_renderer_sample_count(ORIGINAL_DEFAULT_MULTI_SAMPLES),
        1
    );
    let fixture = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/multisample_cases.txt"
    ));
    for line in fixture.lines() {
        let words = line
            .split_whitespace()
            .map(|word| word.parse::<i32>().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(original_renderer_sample_count(words[0]), words[2] as u32);
        // Entire FUN_14036b620 is integer equality against the raw setting,
        // not the clamped texture count. Rust's existing equality suffices.
        assert_eq!((words[0] == words[1]) as i32, words[3]);
    }
}

#[test]
fn native_depth_draw_cache_reset_matches_conditional_two_qword_clear() {
    let mut bound = [1, 2, 3, 4];
    reset_native_depth_draw_cache_if_bound(&mut bound);
    assert_eq!(bound, [0, 2, 3, 0]);
    let mut unbound = [0, 2, 3, 4];
    reset_native_depth_draw_cache_if_bound(&mut unbound);
    assert_eq!(unbound, [0, 2, 3, 4]);
}

#[test]
fn native_indirect_pointer_helpers_apply_the_original_addends() {
    let mut object = [0; 0x18];
    object[0x10..0x18].copy_from_slice(&0x1234_5678_9abc_def0_u64.to_le_bytes());
    assert_eq!(
        native_indirect_pointer_plus_0x58(&object).unwrap(),
        0x1234_5678_9abc_df48
    );
    assert_eq!(
        native_indirect_pointer_plus_0x2d8(&object).unwrap(),
        0x1234_5678_9abc_e1c8
    );
    assert!(native_indirect_pointer_plus_0x58(&object[..0x17]).is_err());
}

#[test]
fn native_optional_parameter_writes_match_the_guard_and_call_order() {
    let values = NativeOptionalParameterValues {
        gate_at_0x38: 0,
        indirect_gate_at_0x78: 0,
        word_at_0x90: 0x1111_2222,
        words_at_0x94: [0x3333_4444, 0x5555_6666, 0x7777_8888, 0x9999_aaaa],
        word_at_0x8c: 0xbbbb_cccc,
    };
    let mut slots = [[0xdead_beef; 4]; 5];
    assert!(write_native_optional_parameter_values(&mut slots, [0, 1, 2], values).unwrap());
    assert_eq!(slots[1], [0x1111_2222, 0, 0, 0]);
    assert_eq!(slots[2], values.words_at_0x94);
    assert_eq!(slots[3], [0xbbbb_cccc, 0, 0, 0]);
    let before = slots;
    assert!(!write_native_optional_parameter_values(
        &mut slots,
        [0, 1, 2],
        NativeOptionalParameterValues {
            gate_at_0x38: 1,
            ..values
        }
    )
    .unwrap());
    assert_eq!(slots, before);
    assert!(!write_native_optional_parameter_values(
        &mut slots,
        [0, 1, 2],
        NativeOptionalParameterValues {
            indirect_gate_at_0x78: 1,
            ..values
        }
    )
    .unwrap());
    assert_eq!(slots, before);
}

#[test]
fn native_single_resource_bind_wraps_one_value_without_changing_arguments() {
    let mut observed = None;
    call_native_resource_array_bind_with_one(
        11,
        22,
        33,
        0x4455,
        |context, count, values, extra, flags| {
            observed = Some((context, count, *values, extra, flags));
        },
    );
    assert_eq!(observed, Some((11, 1, [22], 33, 0x4455)));
}

#[test]
fn native_parameter_slot_writers_preserve_word_layout_and_order() {
    let entries = vec![(2, [1, 2, 3, 4]), (1, [5, 6, 7, 8]), (2, [9, 10, 11, 12])];
    let block = ParameterBlock {
        entries,
        value_count_word: 0xa500_0003,
        id_count_word: 3,
        value_capacity_word: 0x300,
        id_capacity_word: 0x300,
    };
    let mut slots = [[0xdead_beef; 4]; 5];
    block.write_entries_to_slots(&mut slots).unwrap();
    assert_eq!(slots[0], [0xdead_beef; 4]);
    assert_eq!(slots[2], [5, 6, 7, 8]);
    assert_eq!(slots[3], [9, 10, 11, 12]);
    assert_eq!(slots[4], [0xdead_beef; 4]);

    write_native_parameter_u64(&mut slots, 1, 0xfedc_ba98_7654_3210).unwrap();
    assert_eq!(slots[2], [0, 0, 0x7654_3210, 0xfedc_ba98]);
    write_native_parameter_repeated_u32(&mut slots, 2, 0x8000_1234).unwrap();
    assert_eq!(slots[3], [0x8000_1234; 4]);
    write_native_parameter_4_words(
        &mut slots,
        3,
        [0x1020_3040, 0x5060_7080, 0x90a0_b0c0, 0xd0e0_f000],
    )
    .unwrap();
    assert_eq!(
        slots[4],
        [0x1020_3040, 0x5060_7080, 0x90a0_b0c0, 0xd0e0_f000]
    );
    write_native_parameter_u32(&mut slots, 0, 0xdead_cafe).unwrap();
    assert_eq!(slots[1], [0xdead_cafe, 0, 0, 0]);
    write_native_parameter_u32(&mut slots, -1, 0x1234_5678).unwrap();
    assert_eq!(slots[0], [0x1234_5678, 0, 0, 0]);

    let before = slots;
    assert!(write_native_parameter_u64(&mut slots, -2, 1).is_err());
    assert_eq!(slots, before);
    let too_short = ParameterBlock {
        entries: vec![(0, [1; 4])],
        value_count_word: 2,
        id_count_word: 1,
        value_capacity_word: 0x100,
        id_capacity_word: 0x100,
    };
    assert!(too_short.write_entries_to_slots(&mut slots).is_err());
    assert_eq!(slots, before);
}

#[test]
fn native_batch_row_first_qword_matches_the_native_accessor() {
    let mut row = [0; 14];
    row[0] = 0x7654_3210;
    row[1] = 0xfedc_ba98;
    assert_eq!(
        first_qword_of_native_surface_batch_row(&row),
        0xfedc_ba98_7654_3210
    );
    let mut rows = NativeSurfaceBatchRowList::with_capacity(1, 0).unwrap();
    let index = rows.push_zeroed_row(false, true).unwrap();
    *rows.row_mut(index).unwrap() = row;
    assert_eq!(rows.first_qword(index), Some(0xfedc_ba98_7654_3210));
}

#[test]
fn constructor_range_matches_original_qword_store() {
    let cases = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/model_info_default_cases.txt"
    ));
    let mut checked = 0;
    for line in cases.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 3);
        assert_eq!(u32::from_str_radix(fields[1], 16).unwrap(), 0x3fff0010);
        assert_eq!(
            u32::from_str_radix(fields[2], 16).unwrap(),
            INITIAL_MODEL_MAX_VISIBLE_RANGE.to_bits()
        );
        checked += 1;
    }
    assert_eq!(checked, 4);
}

#[test]
fn parameter_float_matches_original_global_defaults_and_overrides() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parm_float_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        let u = |i: usize| fields[i].parse::<u32>().unwrap();
        if fields[0] == "copy" {
            let source = [u(1), u(2), u(3), u(4)];
            let copied = source;
            assert_eq!(copied, source);
        } else {
            let mut defaults = [[0; 4]; 12];
            defaults[u(1) as usize][0] = u(3);
            let overrides: Vec<_> = (0..u(4) as usize)
                .map(|i| (u(5 + i * 2) as u16, [u(6 + i * 2), 0, 0, 0]))
                .collect();
            assert_eq!(
                parameter_float("emissiveintensity", u(1), u(2), &defaults, &overrides)
                    .unwrap()
                    .to_bits(),
                u(fields.len() - 1),
                "{line}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 305);
    assert_eq!(
        parameter_float("bad", 0, 1, &[[0; 4]], &[]).unwrap_err(),
        "idParmBlock::GetFloat: bad is not a vector parm"
    );
    assert!(parameter_float("missing", 1, 0, &[[0; 4]], &[]).is_err());
}

#[test]
fn parameter_removal_matches_original_paired_arrays() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/parm_clear_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<u32> = line
            .split_whitespace()
            .map(|value| value.parse().unwrap())
            .collect();
        let id = fields[0];
        let count = fields[2] as usize;
        let slot = |base: usize| {
            (
                fields[base] as u16,
                std::array::from_fn(|lane| fields[base + 1 + lane]),
            )
        };
        let mut overrides: Vec<_> = (0..count).map(|index| slot(3 + index * 5)).collect();
        let capacity = overrides.capacity();
        remove_parameter(&mut overrides, id);
        assert_eq!(overrides.capacity(), capacity, "{line}");
        let base = 3 + count * 5;
        assert_eq!(fields[base] & 0xffffff, overrides.len() as u32, "{line}");
        assert_eq!(
            fields[base + 1] & 0xffffff,
            overrides.len() as u32,
            "{line}"
        );
        let expected: Vec<_> = (0..fields[base + 2] as usize)
            .map(|index| slot(base + 3 + index * 5))
            .collect();
        assert_eq!(overrides, expected, "{line}");
        assert_eq!(fields.len(), base + 3 + expected.len() * 5);
        if id < 6 {
            let defaults = [[0x3e800000, 0, 0, 0]; 6];
            assert_eq!(
                parameter_float("test", id, 0, &defaults, &overrides)
                    .unwrap()
                    .to_bits(),
                0x3e800000
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 168);
}

#[test]
fn binding_compaction_and_selection_match_native() {
    let mut operations = [0; 3];
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/material_binding_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        let u = |index: usize| fields[index].parse::<u32>().unwrap();
        match fields[0] {
            "D" | "W" => {
                let count = u(4) as usize;
                let whitelist_count = u(5) as usize;
                let whitelist: Vec<_> = (0..whitelist_count).map(|i| u(6 + i)).collect();
                let base = 6 + whitelist_count;
                let slot = |offset: usize| {
                    (
                        u(offset) as u16,
                        std::array::from_fn(|lane| u(offset + 1 + lane)),
                    )
                };
                let mut block = ParameterBlock {
                    entries: (0..count).map(|i| slot(base + 5 * i)).collect(),
                    value_count_word: u(1),
                    id_count_word: u(2),
                    value_capacity_word: (u(3) << 8) | 0x13,
                    id_capacity_word: (u(3) << 8) | 0x3f,
                };
                let capacity = block.entries.capacity();
                if fields[0] == "D" {
                    block.deduplicate().unwrap();
                } else {
                    block.filter(&whitelist).unwrap();
                }
                let expected = base + count * 5;
                assert_eq!(block.value_count_word, u(expected), "{line}");
                assert_eq!(block.id_count_word, u(expected + 1), "{line}");
                let slots: Vec<_> = (0..u(expected + 2) as usize)
                    .map(|i| slot(expected + 3 + i * 5))
                    .collect();
                assert_eq!(block.entries, slots, "{line}");
                assert_eq!(block.entries.capacity(), capacity);
                assert_eq!(block.value_capacity_word, (u(3) << 8) | 0x13);
                assert_eq!(block.id_capacity_word, (u(3) << 8) | 0x3f);
                if let Some(&(id, value)) = block.entries.last() {
                    let defaults = vec![[0; 4]; usize::from(id) + 1];
                    assert_eq!(
                        block
                            .float("native", u32::from(id), 0, &defaults)
                            .unwrap()
                            .to_bits(),
                        value[0]
                    );
                }
                operations[0] += 1;
            }
            "S" => {
                let group = material_parameter_group(u(1) as u8, fields[2].parse().unwrap());
                if u(3) != 0 {
                    assert_eq!(group, MaterialParameterGroup::Base);
                } else if let MaterialParameterGroup::Effect(selector) = group {
                    assert_eq!(i32::from(selector) * 32, fields[4].parse::<i32>().unwrap());
                } else {
                    panic!("{line}");
                }
                operations[1] += 1;
            }
            "P" => operations[2] += 1,
            _ => panic!("{line}"),
        }
    }
    assert_eq!(operations, [630, 768, 65536]);
}

#[test]
fn prepared_material_groups_feed_the_existing_property_getter() {
    let block = |word| ParameterBlock {
        entries: vec![(0, [word, 0, 0, 0])],
        value_count_word: 1,
        id_count_word: 1,
        value_capacity_word: 256,
        id_capacity_word: 256,
    };
    let mut material = MaterialParameterBlocks {
        base: block(0x3f000000),
        effects: vec![block(0x3f800000)],
        flags_49: 0x40,
    };
    assert_eq!(
        material
            .selected(-1)
            .unwrap()
            .float("value", 0, 0, &[[0; 4]])
            .unwrap(),
        0.5
    );
    assert_eq!(
        material
            .selected(0)
            .unwrap()
            .float("value", 0, 0, &[[0; 4]])
            .unwrap(),
        1.0
    );
    assert!(material.selected(-2).is_err());
    assert!(material.selected(1).is_err());
    material.flags_49 = 0;
    assert_eq!(
        material
            .selected(127)
            .unwrap()
            .float("value", 0, 0, &[[0; 4]])
            .unwrap(),
        0.5
    );
    let mut unsupported = block(0);
    unsupported.value_capacity_word = 0;
    assert!(unsupported.deduplicate().is_err());
    assert!(unsupported.filter(&[]).is_err());
}

fn lod_view() -> NativeLodViewParameters {
    NativeLodViewParameters {
        primary_position: [0.0; 3],
        alternate_position: [0.0; 3],
        use_alternate_position: false,
        fov_sin_half_squared: 1.0,
        level_distances_squared: [9.0, 36.0, 144.0, 400.0],
        far_distance_squared: 900.0,
        force_visible_distance_squared: 10_000.0,
    }
}

#[test]
fn native_lod_fov_scale_uses_original_sine_of_half_fov() {
    assert!(
        (super::native_lod_fov_sin_half_squared(std::f32::consts::FRAC_PI_2) - 0.5).abs() < 1.0e-6
    );
}

#[test]
fn native_lod_mask_table_matches_all_sixteen_original_indices() {
    let expected = [0, 0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 2, 0, 1, 0];
    for mask in 0u32..16 {
        let thresholds = std::array::from_fn(|bit| if mask & (1 << bit) != 0 { 2.0 } else { 0.0 });
        let selected = super::native_lod_select_level(1.0, thresholds, 100.0);
        let expected = if mask == 0 {
            4
        } else {
            expected[mask as usize]
        };
        assert_eq!(selected, expected, "mask={mask:04b}");
    }
}

#[test]
fn native_lod_position_levels_match_mask_and_far_boundaries() {
    let view = lod_view();
    for (x, expected) in [
        (2.0, 0),
        (3.0, 1),
        (6.0, 2),
        (12.0, 3),
        (20.0, 4),
        (30.0, 4),
        (31.0, 5),
    ] {
        let decision = super::native_lod_level_for_position(
            &view,
            NativeLodPositionInput {
                position: [x, 0.0, 0.0],
                current_optimal_level: expected,
            },
        );
        assert_eq!(decision.optimal_level, expected, "x={x}");
        assert!(!decision.needs_update);
    }
    let changed = super::native_lod_level_for_position(
        &view,
        NativeLodPositionInput {
            position: [2.0, 0.0, 0.0],
            current_optimal_level: 5,
        },
    );
    assert_eq!(
        changed,
        NativeLodDecision {
            optimal_level: 0,
            needs_update: true,
        }
    );
}

#[test]
fn native_lod_bounds_uses_largest_horizontal_half_extent() {
    let mut view = lod_view();
    view.primary_position = [5.0, 0.0, 0.0];
    view.level_distances_squared = [3.0, 6.0, 12.0, 20.0];
    view.far_distance_squared = 21.0;
    view.force_visible_distance_squared = 100.0;
    let decision = super::native_lod_level_for_bounds(
        &view,
        NativeLodBoundsInput {
            minimum: [-2.0, -1.0, 0.0],
            maximum: [2.0, 1.0, 0.0],
            flags_1c30: 1,
            flags_1c31: 0,
            bit_1_at_0xd0: false,
            current_optimal_level: 4,
        },
    );
    // Center distance² 25 minus radius² 4 => far² 21 exactly. Native
    // equality stays at level 4; without the extent adjustment it would be 5.
    assert_eq!(decision.optimal_level, 4);
    assert!(!decision.needs_update);
}

#[test]
fn native_lod_bounds_matches_force_visible_and_far_skip_tiers() {
    let mut view = lod_view();
    view.primary_position = [10.0, 0.0, 0.0];
    view.force_visible_distance_squared = 10.0;
    view.level_distances_squared = [1.0, 2.0, 3.0, 4.0];
    view.far_distance_squared = 80.0;
    let bounds = |flags_1c30, flags_1c31, bit_1_at_0xd0| NativeLodBoundsInput {
        minimum: [0.0; 3],
        maximum: [0.0; 3],
        flags_1c30,
        flags_1c31,
        bit_1_at_0xd0,
        current_optimal_level: 0,
    };
    let mut force_boundary_view = lod_view();
    force_boundary_view.primary_position = [2.0, 0.0, 0.0];
    force_boundary_view.force_visible_distance_squared = 4.0;
    force_boundary_view.far_distance_squared = 100.0;
    let at_force_boundary =
        super::native_lod_level_for_bounds(&force_boundary_view, bounds(1, 0, false));
    assert_eq!(at_force_boundary.optimal_level, 0);
    force_boundary_view.primary_position[0] = 2.01;
    let beyond_force_boundary =
        super::native_lod_level_for_bounds(&force_boundary_view, bounds(1, 0, false));
    assert_eq!(beyond_force_boundary.optimal_level, 6);
    assert_eq!(
        super::native_lod_level_for_bounds(&view, bounds(1, 0, false)).optimal_level,
        7
    );
    // The special flag bypasses the force-visible distance gate.
    assert_eq!(
        super::native_lod_level_for_bounds(&view, bounds(0, 4, false)).optimal_level,
        5
    );
    assert_eq!(
        super::native_lod_level_for_bounds(&view, bounds(0, 8, false)).optimal_level,
        5
    );
    assert_eq!(
        super::native_lod_level_for_bounds(&view, bounds(1, 4, false)).optimal_level,
        7
    );
    // FUN_140878d10's bit forces far tiers; equality at far² selects 6.
    view.far_distance_squared = 100.0;
    assert_eq!(
        super::native_lod_level_for_bounds(&view, bounds(1, 0, true)).optimal_level,
        6
    );
}

#[test]
fn native_lod_uses_nearest_of_two_camera_positions() {
    let mut view = lod_view();
    view.primary_position = [100.0, 0.0, 0.0];
    view.alternate_position = [1.0, 0.0, 0.0];
    view.use_alternate_position = true;
    let decision = super::native_lod_level_for_position(
        &view,
        NativeLodPositionInput {
            position: [0.0, 0.0, 0.0],
            current_optimal_level: 0,
        },
    );
    assert_eq!(decision.optimal_level, 0);
}

#[test]
fn native_lod_child_update_args_matches_original_flags() {
    assert_eq!(
        super::native_lod_child_update_args(2, 4, -8),
        Some(super::NativeLodChildUpdate {
            previous_actual: 2,
            actual: 4,
        })
    );
    assert_eq!(super::native_lod_child_update_args(2, 2, -8), None);
    assert_eq!(super::native_lod_child_update_args(2, 4, 8), None);
    assert_eq!(super::native_lod_child_update_args(2, 4, -12), None);
}

#[test]
fn native_lod_state_update_matches_requested_max_clamp() {
    for (previous, optimal, requested_max, expected_actual, changed) in [
        (3, 4, 8, 4, true),
        (4, 7, 4, 4, false),
        (6, 2, 8, 2, true),
        (0, -1, 5, -1, true),
    ] {
        assert_eq!(
            super::native_lod_level_update(previous, optimal, requested_max),
            super::NativeLodLevelUpdate {
                optimal_level: optimal,
                actual_level: expected_actual,
                actual_changed: changed,
            }
        );
    }
}

#[test]
fn native_static_model_lod_slot_matches_original_upper_cap() {
    for (requested, selected) in [(-1, -1), (0, 0), (1, 1), (2, 2), (7, 2)] {
        assert_eq!(super::native_static_model_lod_slot(requested), selected);
    }
}

#[test]
fn native_static_lod_refresh_branch_matches_original() {
    for (flags_d4, actual_level, expected) in [
        (0, 0, false),
        (0, 2, false),
        (1, 0, false),
        (1, 2, true),
        (0x81, -1, true),
    ] {
        assert_eq!(
            super::native_static_model_lod_refresh_required(flags_d4, actual_level),
            expected
        );
    }
}

#[test]
fn native_lod_manager_registration_gates_match_original() {
    for flags_d2 in [0, 0x40] {
        for flags_d4 in [0, 1] {
            for flags_d1 in [0, 1] {
                assert_eq!(
                    super::native_lod_manager_registration_allowed(flags_d2, flags_d4, flags_d1),
                    flags_d2 == 0 && flags_d4 == 0 && flags_d1 == 0
                );
            }
        }
    }
}

#[test]
fn native_lod_model_variant_eligibility_matches_type2_type3_type7_gates() {
    use super::NativeLodModelPredicate as Predicate;
    let no_variants = [None::<u32>, None, None];
    let only_lod0 = [Some(10_u32), None, None];
    let lod2 = [None, None, Some(30_u32)];
    for (predicate, count, single, grouped, expected) in [
        (
            Predicate::Predicate2,
            1,
            &only_lod0,
            &[no_variants][..],
            false,
        ),
        (
            Predicate::Predicate2,
            0x0100_0002,
            &only_lod0,
            &[no_variants][..],
            true,
        ),
        (
            Predicate::Predicate2,
            0x0100_0000,
            &only_lod0,
            &[no_variants][..],
            false,
        ),
        (
            Predicate::Predicate3,
            0,
            &only_lod0,
            &[no_variants][..],
            false,
        ),
        (Predicate::Predicate3, 0, &lod2, &[no_variants][..], true),
        (
            Predicate::Predicate7,
            0,
            &only_lod0,
            &[only_lod0, lod2][..],
            true,
        ),
        (
            Predicate::Predicate7,
            0,
            &only_lod0,
            &[no_variants][..],
            false,
        ),
        (Predicate::Other, 0, &lod2, &[lod2][..], false),
    ] {
        assert_eq!(
            super::native_lod_model_variant_eligible(predicate, count, single, grouped),
            expected
        );
    }
}

#[test]
fn native_lod_child_registration_eligibility_matches_flags_and_callback() {
    assert!(super::native_lod_child_registration_eligible(-4, false));
    assert!(!super::native_lod_child_registration_eligible(12, false));
    assert!(!super::native_lod_child_registration_eligible(-8, false));
    assert!(super::native_lod_child_registration_eligible(0, true));
}

#[test]
fn native_lod_registration_action_matches_model_and_child_branch() {
    use super::NativeLodRegistrationAction as Action;
    for (has_model, model_variant, child, expected) in [
        (true, true, false, Action::BoundsList),
        (true, false, true, Action::BoundsList),
        (false, false, true, Action::PositionList),
        (false, false, false, Action::SetLevel8),
        (true, false, false, Action::NoAction),
        (false, true, false, Action::SetLevel8),
    ] {
        assert_eq!(
            super::native_lod_registration_action(has_model, model_variant, child),
            expected
        );
    }
}

#[test]
fn native_relative_static_model_lod_slot_matches_original_clamps() {
    for (actual, field_0x17c, selected) in [(3, 1, 2), (3, 4, 0), (8, 0, 2), (2, 2, 0)] {
        assert_eq!(
            super::native_relative_static_model_lod_slot(actual, field_0x17c),
            selected
        );
    }
}

#[test]
fn native_lod_model_action_matches_selector_dispatch() {
    use super::{NativeLodModelAction as Action, NativeLodModelPredicate as Predicate};
    assert_eq!(
        super::native_lod_model_action(3, 1, true, Predicate::Predicate2, 2, 0),
        Action::Predicate2 {
            selected_index: 1,
            callback_argument: 4,
        }
    );
    assert_eq!(
        super::native_lod_model_action(3, 1, true, Predicate::Predicate2, 0, 0),
        Action::Predicate2 {
            selected_index: -1,
            callback_argument: 4,
        }
    );
    assert_eq!(
        super::native_lod_model_action(3, 1, true, Predicate::Predicate3, 0, 0),
        Action::Predicate3 {
            static_model_slot: 2,
        }
    );
    assert_eq!(
        super::native_lod_model_action(3, 1, true, Predicate::Predicate7, 0, 4),
        Action::Predicate7 {
            static_model_slot: 2,
            model_count: 4,
        }
    );
    assert_eq!(
        super::native_lod_model_action(3, 1, true, Predicate::Predicate7, 0, 0),
        Action::None
    );
    assert_eq!(
        super::native_lod_model_action(6, 0, true, Predicate::Predicate3, 0, 0),
        Action::None
    );
    assert_eq!(
        super::native_lod_model_action(3, 1, false, Predicate::Predicate3, 0, 0),
        Action::None
    );
}

#[test]
fn native_static_model_has_lod1_or_2_checks_only_nonzero_slots() {
    assert!(!super::native_static_model_has_lod1_or_2(&[
        Some(10_u32),
        None,
        None,
    ]));
    assert!(super::native_static_model_has_lod1_or_2(&[
        None,
        Some(20_u32),
        None,
    ]));
    assert!(super::native_static_model_has_lod1_or_2(&[
        None,
        None,
        Some(30_u32),
    ]));
}

#[test]
fn native_static_model_lod_switch_matches_pointer_and_surface_selection() {
    use super::NativeStaticModelLodSwitch as Switch;
    let model_per_lod = [Some(10_u32), Some(20), Some(30)];
    assert_eq!(
        super::native_static_model_lod_switch(4, &model_per_lod, Some(10)),
        Some(Switch {
            slot: 2,
            selected_model: 30,
            surface_list_byte_offset: 0x20,
            rebuild_surfaces: true,
            callback_vtable_offset: 0x28,
            callback_argument: 0,
        })
    );
    assert_eq!(
        super::native_static_model_lod_switch(1, &model_per_lod, Some(20)),
        None
    );
    assert_eq!(
        super::native_static_model_lod_switch(2, &[Some(10), Some(20), None], Some(10)),
        None
    );
    assert_eq!(
        super::native_static_model_lod_switch(-1, &model_per_lod, Some(10)),
        None
    );
}

#[test]
fn resource_union_matches_original() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/static_resource_bounds_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        let count = fields[0].parse::<usize>().unwrap();
        let f = |index: usize| f32::from_bits(fields[index].parse().unwrap());
        let previous =
            std::array::from_fn(|side| std::array::from_fn(|axis| f(1 + side * 3 + axis)));
        let surfaces: Vec<_> = (0..count)
            .map(|surface| {
                std::array::from_fn(|side| {
                    std::array::from_fn(|axis| f(7 + surface * 6 + side * 3 + axis))
                })
            })
            .collect();
        let actual = accumulate_resource_bounds(previous, &surfaces);
        let expected: [[f32; 3]; 2] = std::array::from_fn(|side| {
            std::array::from_fn(|axis| f(7 + count * 6 + side * 3 + axis))
        });
        assert_eq!(
            actual.map(|row| row.map(f32::to_bits)),
            expected.map(|row| row.map(f32::to_bits)),
            "{line}"
        );
        checked += 1;
    }
    assert_eq!(checked, 73);
}

#[test]
fn helpers_match_installed_executable() {
    let rows = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.01, 1.0],
        [0.0, 0.0, -0.101, 0.0],
    ];
    let state = crate::tome::RenderQueryState {
        render_rows: rows,
        cropped_rows: rows,
        camera_point: [0.0; 3],
        camera_imprecision: [0.6; 3],
        winding_flip: false,
        viewport: [0, 0, 64, 64],
        maximum_depth: 0.0,
        depth_grid: None,
    };
    let query = NoDepthAabbQuery::from_state(&state).unwrap();
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/render_model_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        let u = |i: usize| fields[i].parse::<u32>().unwrap();
        let f = |i: usize| fields[i].parse::<f32>().unwrap();
        match fields[0] {
            "set" | "clear" => {
                let mut flags = u(1);
                if fields[0] == "set" {
                    request_flags(&mut flags, u(2));
                } else {
                    clear_flags(&mut flags, u(2));
                }
                assert_eq!(flags, u(3), "{line}");
            }
            "fade" => {
                let result = distance_fade(
                    f32::from_bits(u(1)),
                    f32::from_bits(u(2)),
                    f32::from_bits(u(3)),
                );
                let expected = f32::from_bits(u(4));
                if expected.is_nan() {
                    assert!(result.is_nan(), "{line}");
                } else {
                    assert_eq!(result.to_bits(), expected.to_bits(), "{line}");
                }
            }
            "tree" => {
                let mut root = VisibilityNode {
                    model_flags: Some(u(1)),
                    children: vec![VisibilityNode {
                        model_flags: None,
                        children: vec![VisibilityNode {
                            model_flags: Some(u(1)),
                            children: vec![],
                        }],
                    }],
                };
                root.set_visible(u(2) != 0);
                assert_eq!(root.model_flags, Some(u(3)), "{line}");
                assert_eq!(
                    root.children[0].children[0].model_flags,
                    Some(u(4)),
                    "{line}"
                );
            }
            "patch" => {
                assert_eq!(
                    square_patch_visible(&query, [f(1), f(2), f(3)], f(4), f(5), f(6)).unwrap(),
                    u(7) != 0,
                    "{line}"
                );
            }
            kind => panic!("unknown native model-helper case {kind}"),
        }
        checked += 1;
    }
    assert_eq!(checked, 476);
}

#[test]
fn registry_prefix_matches_installed_original_instructions() {
    let mut count = 0;
    for row in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/model_registry_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<&str> = row.split_whitespace().collect();
        match fields[0] {
            "resource" | "empty" => {
                let bounds = std::array::from_fn(|side| {
                    std::array::from_fn(|axis| {
                        f32::from_bits(fields[1 + side * 3 + axis].parse().unwrap())
                    })
                });
                let actual = if fields[0] == "empty" {
                    static_resource_bounds(None)
                } else {
                    static_resource_bounds(Some(resource_bounds(&bounds)))
                };
                assert_eq!(
                    actual.map(|side| side.map(f32::to_bits)),
                    bounds.map(|side| side.map(f32::to_bits))
                );
                if fields[0] == "resource" {
                    assert!(std::ptr::eq(actual, &bounds));
                }
            }
            "prefix" => {
                let flags = fields[1].parse().unwrap();
                let shape = fields[2].parse().unwrap();
                let region =
                    std::array::from_fn(|axis| f32::from_bits(fields[3 + axis].parse().unwrap()));
                let camera =
                    std::array::from_fn(|axis| f32::from_bits(fields[7 + axis].parse().unwrap()));
                let accepted = model_eligible(flags, shape) && in_model_region(camera, region);
                assert_eq!(accepted, fields[10] == "1", "native registry case {count}");
            }
            _ => panic!("unknown original registry case"),
        }
        count += 1;
    }
    assert_eq!(count, 16_388);
    // A prepared record stays eligible even when pending bit1 is set.
    assert!(model_eligible(5, 0));
    assert!(!model_eligible(1, 0x10));
}

#[test]
fn geometry_matches_installed_original_operations() {
    let mut checked = 0;
    for line in include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/model_geometry_cases.txt"
    ))
    .lines()
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        let bits = |index: usize| fields[index].parse::<u32>().unwrap();
        let value = |index: usize| f32::from_bits(bits(index));
        let vector = |start: usize| std::array::from_fn(|axis| value(start + axis));
        let basis = |start: usize| std::array::from_fn(|axis| vector(start + axis * 3));
        let (actual, expected): (Vec<f32>, Vec<f32>) = match fields[0] {
            "base" => {
                assert_eq!(base_callback() as u32, bits(1));
                checked += 1;
                continue;
            }
            "matrix" => (
                affine_rows(vector(1), basis(4), vector(13))
                    .into_iter()
                    .flatten()
                    .collect(),
                (16..32).map(value).collect(),
            ),
            "bounds" | "static_bounds" => (
                transformed_bounds([vector(1), vector(4)], vector(7), basis(10), vector(19))
                    .into_iter()
                    .flatten()
                    .collect(),
                (22..28).map(value).collect(),
            ),
            "region" => (
                range_region([vector(1), vector(4)], value(7)).to_vec(),
                (8..12).map(value).collect(),
            ),
            kind => panic!("unknown native model geometry case {kind}"),
        };
        assert_eq!(
            actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "{line}"
        );
        checked += 1;
    }
    assert_eq!(checked, 355);
}

#[test]
fn masked_requests_do_not_fill_partially_set_masks() {
    let mut flags = 2;
    request_flags(&mut flags, 6);
    assert_eq!(flags, 2);
    clear_flags(&mut flags, 6);
    assert_eq!(flags, 1);
    clear_flags(&mut flags, 1);
    assert_eq!(flags, 0);
}

#[test]
fn visibility_reaches_children_without_parent_model() {
    let mut root = VisibilityNode {
        model_flags: None,
        children: vec![VisibilityNode {
            model_flags: Some(0),
            children: vec![],
        }],
    };
    root.set_visible(true);
    assert_eq!(root.children[0].model_flags, Some(3));
    root.set_visible(false);
    assert_eq!(root.children[0].model_flags, Some(1));
}

#[test]
fn fade_matches_endpoints_and_unordered_machine_comparisons() {
    assert_eq!(distance_fade(0.0, 1.0, -1.0), 0.0);
    assert_eq!(distance_fade(0.0, 1.0, 0.5), 0.5);
    assert_eq!(distance_fade(0.0, 1.0, 2.0), 1.0);
    assert!(distance_fade(0.0, 0.0, 0.0).is_nan());
}

#[test]
fn surface_reduction_clears_only_derived_bits_and_ors_all_surfaces() {
    let a = SurfaceFlags {
        byte_48: 0,
        byte_4b: 4,
        byte_4c: 0,
        parameter_14229d460: 0.0,
    };
    let b = SurfaceFlags {
        byte_48: 1,
        byte_4b: 0,
        byte_4c: 0x41,
        parameter_14229d460: 0.002,
    };
    assert_eq!(reduce_surface_flags([0xff, 0xff], &[]), [0x0f, 0xfe]);
    assert_eq!(reduce_surface_flags([0xff, 0xff], &[a]), [0x0f, 0xfe]);
    assert_eq!(reduce_surface_flags([0, 4], &[b, a]), [0xf0, 5]);
}
