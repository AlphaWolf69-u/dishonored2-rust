use super::*;

fn words(row: &str) -> Vec<u32> {
    row.split_whitespace()
        .map(|v| u32::from_str_radix(v, 16).unwrap())
        .collect()
}

#[test]
fn registry_matches_executed_registration_and_ordered_removal() {
    let mut registry = EnvironmentEffectRegistry::<u32, u32>::default();
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_effect_registry.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        if w[0] != 0 {
            registry = EnvironmentEffectRegistry::default();
        }
        if w[1] == 0 {
            registry.register(w[2]).unwrap();
        } else {
            let registration = (w[3] != 0 && w[4] != 0).then(|| (w[4], (w[3] - 1) as usize));
            registry.remove(&w[2], registration).unwrap();
        }
        let state = registry.state.lock().unwrap();
        let n = w[5] as usize;
        let m = w[6] as usize;
        assert_eq!(w.len(), 7 + n + 2 * m);
        assert_eq!(state.active, w[7..7 + n], "case {case} active");
        let pending = state
            .retirements
            .pending
            .iter()
            .flat_map(|(view, slot)| [*view, slot.unwrap() as u32])
            .collect::<Vec<_>>();
        assert_eq!(pending, w[7 + n..], "case {case} pending");
        count += 1;
    }
    assert_eq!(count, 512);
}
#[test]
fn effect_binding_matches_executed_view_callbacks() {
    let mut binding = EnvironmentEffectBinding::new(1);
    let mut views: [EnvironmentOverrides<u32>; 2] =
        std::array::from_fn(|_| EnvironmentOverrides::default());
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_effect_bindings.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        assert_eq!(w.len(), 15);
        if w[0] != 0 {
            binding = EnvironmentEffectBinding::new(w[1]);
            views = std::array::from_fn(|_| EnvironmentOverrides::default());
        }
        let view = w[3] as usize;
        binding
            .publish(
                view as u32 + 1,
                &mut views[view],
                w[2] as u8,
                w[4] as u8,
                f32::from_bits(w[5]),
            )
            .unwrap();
        // Word6 is the native FX lifetime flag. 21a190 itself does not test it.
        assert_eq!(
            binding.registration().map(|(view, _)| *view).unwrap_or(0),
            w[7],
            "case {case} owner"
        );
        assert_eq!(
            binding
                .registration()
                .map(|(_, slot)| slot as u32)
                .unwrap_or(u32::MAX),
            w[8],
            "case {case} handle"
        );
        for (index, slots) in views.iter_mut().enumerate() {
            let mut current = Vec::new();
            slots.visit_frame(|env, weight, influence| {
                current.push((*env, weight.to_bits(), influence.to_bits()))
            });
            assert!(current.len() <= 1);
            let observed = current.first().copied().unwrap_or((0, 0, 1f32.to_bits()));
            let at = 9 + index * 3;
            assert_eq!(
                observed,
                (w[at], w[at + 1], w[at + 2]),
                "case {case} view {index}"
            );
        }
        count += 1;
    }
    assert_eq!(count, 512);
    assert_eq!(
        binding.registration().map(|(v, slot)| (*v, slot)),
        binding.into_registration()
    );
}

#[test]
fn retirement_dispatch_matches_executed_live_view_filter() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_effect_retirements.txt")
        .lines()
        .enumerate()
    {
        let w = words(row);
        assert_eq!(w.len(), 40);
        let mut queue = EnvironmentEffectRetirements::default();
        for i in 0..w[1] as usize {
            let at = 6 + i * 2;
            queue.push(w[at], (w[at + 1] != u32::MAX).then_some(w[at + 1] as usize));
        }
        let mut events = Vec::new();
        queue.flush(&w[2..2 + w[0] as usize], |view, slot| {
            events.push((*view, slot.map(|v| v as u32).unwrap_or(u32::MAX)))
        });
        let expected: Vec<_> = (0..w[38] as usize)
            .map(|i| (w[22 + i * 2], w[23 + i * 2]))
            .collect();
        assert_eq!(events, expected, "case {case}");
        assert_eq!(queue.pending.len(), w[39] as usize);
        count += 1;
    }
    assert_eq!(count, 64);
}
