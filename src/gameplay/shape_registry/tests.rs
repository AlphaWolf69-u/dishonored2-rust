use super::*;
use std::collections::HashMap;
struct Store(HashMap<usize, ShapeSignals>);
impl ShapeSignalStore for Store {
    fn signals_mut(&mut self, shape: usize) -> Result<&mut ShapeSignals, String> {
        self.0
            .get_mut(&shape)
            .ok_or("test shape has no live signal resource".into())
    }
}
fn snapshot(registries: &[Option<ShapeRegistry>; 2]) -> Vec<u32> {
    let mut result = Vec::new();
    for registry in registries {
        let Some(registry) = registry else {
            result.push(0);
            continue;
        };
        result.extend([1, u32::from(registry.has_pending_changes())]);
        for list in [&registry.active, &registry.recycled] {
            result.push(list.len() as u32);
            for &id in list {
                let entry = &registry.registrations[id];
                result.extend([
                    id as u32,
                    entry.shape.unwrap_or(0) as u32,
                    u32::from(entry.status),
                    entry.bodies.len() as u32,
                ]);
                result.extend(&entry.bodies);
            }
        }
    }
    result
}
#[test]
fn shape_registry_matches_executed_lifecycle() {
    replay(
        include_str!("../../../tests/fixtures/shape_registry.txt"),
        534,
    );
}
#[test]
fn shape_registry_matches_executed_body_removal() {
    replay(
        include_str!("../../../tests/fixtures/shape_registry_removal.txt"),
        540,
    );
}
fn replay(fixture: &str, expected_count: usize) {
    let mut registries: [Option<ShapeRegistry>; 2] = [None, None];
    let mut store = Store((1..=6).map(|id| (id, ShapeSignals::default())).collect());
    let mut next_shape = 7;
    let mut count = 0;
    for (case, line) in fixture.lines().enumerate() {
        let mut words = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let kind = words.next().unwrap();
        let owner = words.next().unwrap() as usize;
        let shape = words.next().unwrap() as usize;
        let value = words.next().unwrap();
        match kind {
            0 => registries[owner - 1]
                .as_mut()
                .unwrap()
                .register_body(shape, value, &mut store)
                .unwrap(),
            1 => store.signals_mut(shape).unwrap().mutated.emit(
                &(value as u8),
                |_, observer, callback, flags| {
                    assert_eq!(callback, ShapeCallback::Mutated);
                    registries[observer.owner - 1]
                        .as_mut()
                        .expect("callback targets released world")
                        .mutated(observer, *flags)
                        .unwrap();
                },
            ),
            2 => {
                let ShapeSignals { mutated, destroyed } = store.signals_mut(shape).unwrap();
                destroyed.emit(&(), |current, observer, callback, _| {
                    assert_eq!(callback, ShapeCallback::Destroyed);
                    registries[observer.owner - 1]
                        .as_mut()
                        .expect("callback targets released world")
                        .destroyed(observer, shape, mutated, current)
                        .unwrap();
                });
                store.0.remove(&shape);
            }
            3 => {
                assert_eq!(shape, next_shape);
                next_shape += 1;
                assert!(store.0.insert(shape, ShapeSignals::default()).is_none());
            }
            4 => {
                if let Some(registry) = registries[owner - 1].take() {
                    registry.release(&mut store).unwrap();
                }
            }
            5 => {
                assert!(registries[owner - 1].is_none());
                let mut registry = ShapeRegistry::new(owner);
                // Input states include the source's pre-existing empty pool;
                // pending-change processing that populates it is a separate path.
                for id in 0..value as usize {
                    registry.registrations.push(Registration::default());
                    registry.recycled.push(id);
                }
                registries[owner - 1] = Some(registry);
            }
            6 => registries[owner - 1]
                .as_mut()
                .unwrap()
                .remove_body(shape, value, &mut store)
                .unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(
            snapshot(&registries),
            words.collect::<Vec<_>>(),
            "case {case}"
        );
        count += 1;
    }
    assert_eq!(count, expected_count);
}
