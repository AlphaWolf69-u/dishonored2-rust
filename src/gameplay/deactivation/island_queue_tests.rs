use super::*;

#[test]
fn island_queue_matches_executed_assignment_wake_and_duplicate_rules() {
    let mut cases = 0;
    for row in include_str!("../../../tests/fixtures/deactivation_island_queue.txt").lines() {
        let mut w = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let case = w.next().unwrap();
        let mut bodies = BodyStorage::new(8).unwrap();
        for _ in 1..8 {
            let id = bodies.reserve(&mut []).unwrap();
            bodies.get_mut(id).unwrap().motion_id = w.next().unwrap();
        }
        let states = (0..4)
            .map(|i| {
                let mut state = MotionDeactivation::new(i, 0);
                state.set_island(w.next().unwrap() as i32);
                state
            })
            .collect();
        let mut world = DeactivationWorld::new(states);
        world.islands = (0..3)
            .map(|id| {
                Some(DeactivatedIsland {
                    id,
                    wake: w.next().unwrap() as u8,
                    queued: w.next().unwrap() as u8,
                    ..Default::default()
                })
            })
            .collect();
        let n = w.next().unwrap() as usize;
        world.activation_queue = w.by_ref().take(n).collect();
        let steps = w.next().unwrap();
        for step in 0..steps {
            let body = w.next().unwrap();
            let flags = w.next().unwrap() as u8;
            world.queue_island(&bodies, body, flags).unwrap();
            let actual: Vec<_> = world
                .islands
                .iter()
                .flat_map(|i| {
                    let i = i.as_ref().unwrap();
                    [u32::from(i.wake), u32::from(i.queued)]
                })
                .collect();
            assert_eq!(
                actual,
                w.by_ref().take(6).collect::<Vec<_>>(),
                "islands case{case} step{step}"
            );
            let n = w.next().unwrap() as usize;
            assert_eq!(
                world.activation_queue,
                w.by_ref().take(n).collect::<Vec<_>>(),
                "queue case{case} step{step}"
            );
        }
        assert_eq!(w.next(), None);
        cases += 1;
    }
    assert_eq!(cases, 512);
}
