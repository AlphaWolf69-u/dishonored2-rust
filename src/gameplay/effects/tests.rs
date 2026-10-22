use super::environment::*;
use super::{curve::*, parameters::*, timing::*};
use super::{groups::*, pool::EffectPoolLinks};
fn words(row: &str) -> Vec<u32> {
    row.split_whitespace()
        .map(|v| u32::from_str_radix(v, 16).unwrap())
        .collect()
}

#[test]
fn group_offsets_match_executed_group_start_handoff() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/effect_group_offsets.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        assert_eq!(w.len(), 15);
        let f = |i| f32::from_bits(w[i]);
        let ranges = EffectTimingRanges {
            delay: [f(2), f(3)],
            fade_in: [f(4), f(5)],
            duration: [f(6), f(7)],
            fade_out: [f(8), f(9)],
        };
        let mut seed = w[0];
        assert_eq!(
            sample_offsets(&ranges, f(1), &mut seed).map(f32::to_bits),
            w[10..14],
            "case {case}"
        );
        assert_eq!(seed, w[14], "case {case} seed");
        count += 1;
    }
    assert_eq!(count, 512);
}

fn environment_words(e: &EnvironmentBlendEffect) -> Vec<u32> {
    let mut actual = timing_words(&e.timing, 0)[..9].to_vec();
    actual.extend([
        e.parameter.scale.to_bits(),
        e.parameter.curve_mix.to_bits(),
        e.initial_main.to_bits(),
        e.last_main.to_bits(),
        e.blend.to_bits(),
        u32::from(e.view_mask),
        u32::from(e.active),
    ]);
    actual
}

#[test]
fn environment_chain_matches_executed_retirement_and_tick_order() {
    let mut pool = EffectPoolLinks {
        next: vec![],
        free_head: u16::MAX,
        allocated_count: 0,
    };
    let mut head = u16::MAX;
    let mut effects = Vec::new();
    let mut seed = 0;
    let mut cases = (0, 0);
    for (case, row) in include_str!("../../../tests/fixtures/effect_environment_chain.txt")
        .lines()
        .enumerate()
    {
        let (command, data) = row.split_once(' ').unwrap();
        let w = words(data);
        let f = |i| f32::from_bits(w[i]);
        let expected = match command {
            "RESET" => {
                assert_eq!(w.len(), 186);
                seed = w[0];
                head = w[2] as u16;
                pool = EffectPoolLinks {
                    next: w[5..11].iter().map(|v| *v as u16).collect(),
                    free_head: w[3] as u16,
                    allocated_count: w[4] as u16,
                };
                effects.clear();
                for i in 0..6 {
                    let at = 17 + i * 12;
                    let d = EnvironmentEffectDefinition {
                        looping: w[at] != 0,
                        keep_duration_on_stop: w[at + 1] != 0,
                        timing: EffectTimingRanges {
                            delay: [f(at + 2), f(at + 3)],
                            fade_in: [f(at + 4), f(at + 5)],
                            duration: [f(at + 6), f(at + 7)],
                            fade_out: [f(at + 8), f(at + 9)],
                        },
                        scale: [f(at + 10), f(at + 11)],
                        ..Default::default()
                    };
                    let mut e =
                        EnvironmentBlendEffect::new(d, [0.; 4], w[1] as u8, &mut seed).unwrap();
                    e.active = w[11 + i] != 0;
                    effects.push(Some(e));
                }
                assert_eq!(seed, w[89], "case {case} initial seed");
                cases.0 += 1;
                &w[90..]
            }
            "TICK" => {
                assert_eq!(w.len(), 116);
                let mut retired = Vec::new();
                let updated = update_environment_chain(
                    &mut pool,
                    &mut head,
                    &mut effects,
                    f(0),
                    w[1] as u8,
                    &mut seed,
                    |id, _| {
                        retired.push(u32::from(id));
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(
                    [
                        updated,
                        seed,
                        u32::from(head),
                        u32::from(pool.free_head),
                        u32::from(pool.allocated_count)
                    ],
                    w[2..7],
                    "case {case} owner"
                );
                assert_eq!(
                    pool.next.iter().map(|v| u32::from(*v)).collect::<Vec<_>>(),
                    w[7..13],
                    "case {case} links"
                );
                assert_eq!(retired.len(), w[13] as usize);
                assert_eq!(
                    retired,
                    w[14..14 + retired.len()],
                    "case {case} retirement order"
                );
                cases.1 += 1;
                &w[20..]
            }
            _ => panic!("unknown chain command {command}"),
        };
        let actual = effects
            .iter()
            .flat_map(|e| environment_words(e.as_ref().unwrap()))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "case {case} states");
    }
    assert_eq!(cases, (64, 768));
}

#[test]
fn pool_acquire_matches_executed_chain_transfer() {
    let mut cases = 0;
    for (case, row) in include_str!("../../../tests/fixtures/effect_pool_acquire.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        assert_eq!(w.len(), 39);
        let mut free = w[0] as u16;
        let mut head = w[1] as u16;
        let mut count = w[2] as u16;
        let mut links = w[3..19].iter().map(|v| *v as u16).collect::<Vec<_>>();
        let success = super::pool::acquire(&mut free, &mut head, &mut links, &mut count).unwrap();
        let mut actual = vec![
            u32::from(success),
            u32::from(free),
            u32::from(head),
            u32::from(count),
        ];
        actual.extend(links.into_iter().map(u32::from));
        assert_eq!(actual, w[19..], "case {case}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}

#[test]
fn environment_defaults_match_executed_complete_constructor() {
    let d = EnvironmentEffectDefinition::default();
    // Enabled is descriptor metadata; all executable blend inputs follow it.
    let mut actual = vec![1, u32::from(d.looping), u32::from(d.keep_duration_on_stop)];
    actual.extend(
        [
            d.timing.delay,
            d.timing.fade_in,
            d.timing.duration,
            d.timing.fade_out,
            d.scale,
        ]
        .into_iter()
        .flatten()
        .map(f32::to_bits),
    );
    for pair in [d.duration, d.fade_in, d.fade_out] {
        for c in [pair.maximum, pair.minimum] {
            actual.extend([c.times.len() as u32, c.values.len() as u32]);
            actual.extend(c.times.into_iter().chain(c.values).map(f32::to_bits));
        }
    }
    assert_eq!(
        actual,
        words(include_str!(
            "../../../tests/fixtures/effect_environment_defaults.txt"
        ))
    );
}
#[test]
fn parameter_random_matches_executed_state_and_samples() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/effect_parameter_random.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        assert_eq!(w.len(), 6);
        let mut seed = w[0];
        let value =
            EffectParameterSample::new([f32::from_bits(w[1]), f32::from_bits(w[2])], &mut seed);
        assert_eq!(
            [value.scale.to_bits(), value.curve_mix.to_bits(), seed],
            [w[3], w[4], w[5]],
            "case {case}"
        );
        count += 1;
    }
    assert_eq!(count, 512);
}
fn timing_words(t: &EffectTimings, seed: u32) -> [u32; 10] {
    [
        t.duration_offset.to_bits(),
        t.fade_out_offset.to_bits(),
        t.delay_end.to_bits(),
        t.fade_in_rate.to_bits(),
        t.fade_in_end.to_bits(),
        t.duration_rate.to_bits(),
        t.duration_end.to_bits(),
        t.fade_out_rate.to_bits(),
        u32::from(t.fading_out),
        seed,
    ]
}
#[test]
fn timing_matches_executed_initialize_restart_and_stop() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/effect_timings.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        assert_eq!(w.len(), 46);
        let f = |i| f32::from_bits(w[i]);
        let ranges = EffectTimingRanges {
            delay: [f(1), f(2)],
            fade_in: [f(3), f(4)],
            duration: [f(5), f(6)],
            fade_out: [f(7), f(8)],
        };
        let mut seed = w[0];
        let mut timing = EffectTimings::new(&ranges, [f(9), f(10), f(11), f(12)], &mut seed);
        assert_eq!(
            timing_words(&timing, seed),
            w[16..26],
            "case {case} initial"
        );
        timing.restart(&ranges, f(13), &mut seed);
        assert_eq!(
            timing_words(&timing, seed),
            w[26..36],
            "case {case} restart"
        );
        timing.begin_fade_out(w[14] != 0, f(15));
        assert_eq!(timing_words(&timing, seed), w[36..46], "case {case} stop");
        timing.begin_fade_out(w[14] != 0, f(15) + 99.);
        assert_eq!(
            timing_words(&timing, seed),
            w[36..46],
            "case {case} repeated stop"
        );
        count += 1;
    }
    assert_eq!(count, 512);
}
#[test]
fn float_curve_pairs_match_executed_samples() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/effect_float_curves.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        assert_eq!(w.len(), 39);
        let values = |start: usize, n: usize| {
            w[start..start + n]
                .iter()
                .map(|v| f32::from_bits(*v))
                .collect()
        };
        let curves = EffectFloatCurvePair {
            maximum: EffectFloatCurve {
                times: values(6, w[0] as usize),
                values: values(10, w[1] as usize),
            },
            minimum: EffectFloatCurve {
                times: values(22, w[2] as usize),
                values: values(26, w[3] as usize),
            },
        };
        let actual = curves
            .sample(f32::from_bits(w[4]), f32::from_bits(w[5]))
            .unwrap();
        assert_eq!(actual.to_bits(), w[38], "case {case}");
        count += 1;
    }
    assert_eq!(count, 512);
}

#[test]
fn environment_blend_matches_executed_controller_sequences() {
    let mut effect: Option<EnvironmentBlendEffect> = None;
    let mut seed = 0;
    let mut starts = 0;
    let mut ticks = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_blend_effect.txt")
        .lines()
        .enumerate()
    {
        let (command, data) = row.split_once(' ').unwrap();
        let w = words(data);
        let f = |i| f32::from_bits(w[i]);
        let expected = match command {
            "INIT" => {
                assert_eq!(w.len(), 72);
                let curve = |index: usize| {
                    let side = |offset: usize, enabled: bool| EffectFloatCurve {
                        times: if enabled { vec![0., 1.] } else { vec![] },
                        values: if enabled {
                            w[offset..offset + 6]
                                .iter()
                                .map(|v| f32::from_bits(*v))
                                .collect()
                        } else {
                            vec![]
                        },
                    };
                    EffectFloatCurvePair {
                        maximum: side(19 + index * 12, true),
                        minimum: side(25 + index * 12, w[18] & (1 << index) != 0),
                    }
                };
                let definition = EnvironmentEffectDefinition {
                    looping: w[1] != 0,
                    keep_duration_on_stop: w[2] != 0,
                    timing: EffectTimingRanges {
                        delay: [f(4), f(5)],
                        fade_in: [f(6), f(7)],
                        duration: [f(8), f(9)],
                        fade_out: [f(10), f(11)],
                    },
                    scale: [f(16), f(17)],
                    duration: curve(0),
                    fade_in: curve(1),
                    fade_out: curve(2),
                };
                seed = w[0];
                effect = Some(
                    EnvironmentBlendEffect::new(
                        definition,
                        [f(12), f(13), f(14), f(15)],
                        w[3] as u8,
                        &mut seed,
                    )
                    .unwrap(),
                );
                starts += 1;
                &w[55..]
            }
            "TICK" => {
                assert_eq!(w.len(), 19);
                effect
                    .as_mut()
                    .unwrap()
                    .update(f(0), w[1] as u8, &mut seed)
                    .unwrap();
                ticks += 1;
                &w[2..]
            }
            "STOP" => {
                assert_eq!(w.len(), 18);
                effect.as_mut().unwrap().stop(f(0));
                ticks += 1;
                &w[1..]
            }
            _ => panic!("unknown command {command}"),
        };
        let e = effect.as_ref().unwrap();
        let mut actual = timing_words(&e.timing, seed)[..9].to_vec();
        actual.extend([
            e.parameter.scale.to_bits(),
            e.parameter.curve_mix.to_bits(),
            e.initial_main.to_bits(),
            e.last_main.to_bits(),
            e.blend.to_bits(),
            u32::from(e.view_mask),
            u32::from(e.active),
            seed,
        ]);
        assert_eq!(actual, expected, "case {case}: {command}");
    }
    assert_eq!((starts, ticks), (64, 940));
}
