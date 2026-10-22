use super::super::environment_images::ImageOverrides;
use super::*;
use crate::gameplay::effects::timing::EffectTimingRanges;

#[test]
fn default_pool_matches_executed_constructor() {
    let runtime = EnvironmentEffectRuntime::<u32, u32>::default();
    let mut actual = vec![
        runtime.effects.len() as u32,
        runtime.pool.next.len() as u32,
        u32::from(runtime.pool.free_head),
        u32::from(runtime.pool.allocated_count),
    ];
    actual.extend(runtime.pool.next.into_iter().map(u32::from));
    let expected = include_str!("../../../tests/fixtures/effect_environment_pool_defaults.txt")
        .split_whitespace()
        .map(|v| u32::from_str_radix(v, 16).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn empty_runtime() -> EnvironmentEffectRuntime<u32, u32> {
    EnvironmentEffectRuntime::new(EffectPoolLinks {
        next: vec![1, 2, 3, 4, 5, u16::MAX],
        free_head: 0,
        allocated_count: 0,
    })
    .unwrap()
}
fn snapshot(
    runtime: &EnvironmentEffectRuntime<u32, u32>,
    head: u16,
    seed: u32,
    slots: &mut EnvironmentOverrides<u32>,
) -> Vec<u32> {
    let mut result = vec![
        u32::from(runtime.pool.free_head),
        u32::from(head),
        u32::from(runtime.pool.allocated_count),
    ];
    result.extend(runtime.pool.next.iter().copied().map(u32::from));
    result.push(seed);
    let ids = runtime.registry.with_active(|ids| ids.to_vec()).unwrap();
    result.push(ids.len() as u32);
    result.extend(ids.into_iter().map(u32::from));
    for (effect, binding) in runtime.effects.iter().zip(&runtime.bindings) {
        if let Some(e) = effect {
            let t = &e.timing;
            result.extend(
                [
                    t.duration_offset,
                    t.fade_out_offset,
                    t.delay_end,
                    t.fade_in_rate,
                    t.fade_in_end,
                    t.duration_rate,
                    t.duration_end,
                    t.fade_out_rate,
                ]
                .map(f32::to_bits),
            );
            result.push(u32::from(t.fading_out));
            result.extend(
                [
                    e.parameter.scale,
                    e.parameter.curve_mix,
                    e.initial_main,
                    e.last_main,
                    e.blend,
                ]
                .map(f32::to_bits),
            );
            result.extend([u32::from(e.view_mask), u32::from(e.active)]);
        } else {
            result.extend([0; 16]);
        }
        let registration = binding
            .as_ref()
            .and_then(EnvironmentEffectBinding::registration);
        result.extend(registration.map_or([0, u32::MAX], |(view, slot)| [*view, slot as u32]));
    }
    let mut images = ImageOverrides::default();
    slots.visit_frame(|env, weight, influence| {
        images.add_environment(Some(*env), weight, influence)
    });
    let blend = images.finish();
    for (image, weight) in blend.images.iter().zip(blend.weights) {
        result.extend([image.unwrap_or(0), weight.to_bits()]);
    }
    result.push(if blend.is_active() { 2 } else { 1 });
    result
}

#[test]
fn complete_lifetime_and_view_publication_match_executed_pipeline() {
    let mut runtime = empty_runtime();
    let mut slots = EnvironmentOverrides::default();
    let mut head = u16::MAX;
    let mut seed = 0;
    let mut rows = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_effect_pipeline.txt")
        .lines()
        .enumerate()
    {
        let (command, rest) = row.split_once(' ').unwrap();
        let w = rest
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let f = |i| f32::from_bits(w[i]);
        let input_words = match command {
            "RESET" => {
                runtime = empty_runtime();
                slots = EnvironmentOverrides::default();
                head = u16::MAX;
                seed = w[0];
                1
            }
            "START" => {
                let definition = EnvironmentEffectDefinition {
                    looping: w[1] != 0,
                    keep_duration_on_stop: w[2] != 0,
                    timing: EffectTimingRanges {
                        delay: [f(4), f(5)],
                        fade_in: [f(6), f(7)],
                        duration: [f(8), f(9)],
                        fade_out: [f(10), f(11)],
                    },
                    scale: [f(12), f(13)],
                    ..Default::default()
                };
                let started = runtime
                    .start(
                        &mut head,
                        w[0],
                        definition,
                        [f(14), f(15), f(16), f(17)],
                        w[3] as u8,
                        &mut seed,
                    )
                    .unwrap();
                assert_eq!(u32::from(started), w[18], "case {case} admission");
                19
            }
            "TICK" => {
                assert_eq!(
                    runtime
                        .update_group(&mut head, f(0), w[1] as u8, &mut seed)
                        .unwrap(),
                    w[2],
                    "case {case} updated"
                );
                3
            }
            "PUBLISH" => {
                runtime.publish(1, w[0] as u8, &mut slots).unwrap();
                1
            }
            "STOP" => {
                runtime.stop(w[0] as u16, f(1)).unwrap();
                2
            }
            "FLUSH" => {
                runtime
                    .flush_retirements(&[1], |view, slot| {
                        assert_eq!(*view, 1);
                        slots.retire(slot);
                    })
                    .unwrap();
                0
            }
            _ => panic!("unknown pipeline command {command}"),
        };
        assert_eq!(
            snapshot(&runtime, head, seed, &mut slots),
            w[input_words..],
            "case {case}: {command}"
        );
        rows += 1;
    }
    assert_eq!(rows, 560);
}
