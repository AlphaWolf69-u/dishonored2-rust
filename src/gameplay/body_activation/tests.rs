use super::*;
use crate::gameplay::{
    body_storage::{activation_snapshot, activation_test_state},
    deactivation::{DeactivatedIsland, MotionDeactivation},
};

struct Callbacks {
    events: Vec<u32>,
    mode: u32,
    target: u32,
}
impl ActivationCallbacks for Callbacks {
    fn notify(
        &mut self,
        world: &mut DeactivationWorld,
        bodies: &mut BodyStorage,
        _: &mut MotionStorage,
        ids: &[u32],
        activated: bool,
    ) -> Result<(), String> {
        self.events
            .extend([0xa0, u32::from(activated), ids.len() as u32]);
        self.events.extend_from_slice(ids);
        if self.mode & 1 != 0 {
            bodies.get_mut(self.target).unwrap().broadphase_id = u32::MAX;
        }
        if self.mode & 2 != 0 {
            let motion = bodies.get(self.target).unwrap().motion_id;
            world.state_mut(motion).unwrap().0[11] = 42;
        }
        if self.mode & 4 != 0 {
            let next = bodies.get(self.target).unwrap().next_attached_body_id;
            let following = bodies.get(next).unwrap().next_attached_body_id;
            bodies.get_mut(self.target).unwrap().next_attached_body_id = following;
        }
        Ok(())
    }
    fn update_broadphase(
        &mut self,
        _: &mut BodyStorage,
        ids: &[u32],
        stride: u32,
    ) -> Result<(), String> {
        self.events.extend([0xb0, stride, ids.len() as u32]);
        self.events.extend_from_slice(ids);
        Ok(())
    }
}

fn list(words: &mut impl Iterator<Item = u32>) -> Vec<u32> {
    let count = words.next().unwrap() as usize;
    words.take(count).collect()
}

#[test]
fn world_activation_matches_complete_executed_state_and_callback_sequence() {
    check_world_activation(false);
}

#[test]
fn world_activation_uses_owned_signals_and_real_pending_marks() {
    check_world_activation(true);
}

fn check_world_activation(connected: bool) {
    let mut count = 0;
    let mut controller = BodyActivationController::default();
    for row in include_str!("../../../tests/fixtures/body_activation.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let phase = words.next().unwrap();
        let deactivate = words.next().unwrap() != 0;
        let preactive = words.next().unwrap();
        let suppress = words.next().unwrap() != 0;
        let mode = words.next().unwrap();
        let static_body = words.next().unwrap() != 0;
        let first = words.next().unwrap();
        let (mut bodies, mut motions) = activation_test_state(&mut words);
        let mut states = Vec::new();
        for i in 0..4 {
            let mut state = MotionDeactivation::new(i, words.next().unwrap() as u8);
            state.0[11] = words.next().unwrap() as u8;
            state.set_island(words.next().unwrap() as i32);
            states.push(state);
        }
        let members = list(&mut words);
        let wake = words.next().unwrap() as u8;
        let queued = words.next().unwrap() as u8;
        let mut world = DeactivationWorld::new(states);
        world.islands.push(Some(DeactivatedIsland {
            id: 0,
            bodies: members,
            wake,
            queued,
            ..Default::default()
        }));
        world.suppress_island_detach = suppress;
        for i in 0..3 {
            if preactive >> i & 1 != 0 {
                let id = bodies.get_by_index(i + 1).unwrap().id;
                bodies.activate_attached(id, &mut motions).unwrap();
            }
        }
        if static_body {
            bodies.get_mut(first).unwrap().flags |= 1;
        }
        let mut callbacks = Callbacks {
            events: Vec::new(),
            mode,
            target: first,
        };
        let mut broadphase = crate::gameplay::broadphase::HybridBroadphase::with_body_capacity(8);
        let result = if connected {
            let mut signals = ActivationSignals::<u32, u32>::default();
            signals.activated.subscribe(10, 20);
            signals.deactivated.subscribe(10, 20);
            let dispatch = |_: &mut crate::gameplay::signal::Signal<u32, u32>,
                            subscriber,
                            callback,
                            owners: ActivationOwners<'_>,
                            ids: &[u32],
                            activated| {
                assert_eq!((subscriber, callback), (10, 20));
                assert_eq!(
                    owners.broadphase.pending_body_range(),
                    None,
                    "notify before the spatial-index handoff"
                );
                callbacks.notify(
                    owners.deactivation,
                    owners.bodies,
                    owners.motions,
                    ids,
                    activated,
                )
            };
            let mut context = ActivationContext {
                signals: &mut signals,
                broadphase: &mut broadphase,
                dispatch,
            };
            controller
                .set(
                    &mut bodies,
                    &mut motions,
                    &mut world,
                    first,
                    deactivate,
                    phase,
                    &mut context,
                )
                .unwrap()
        } else {
            controller
                .set(
                    &mut bodies,
                    &mut motions,
                    &mut world,
                    first,
                    deactivate,
                    phase,
                    &mut callbacks,
                )
                .unwrap()
        };
        let mut actual = vec![u32::from(result)];
        actual.extend(activation_snapshot(&bodies, &motions));
        for state in world.states.iter().flatten() {
            actual.extend([
                u32::from(state.mode()),
                u32::from(state.0[11]),
                state.island() as u32,
            ]);
        }
        let island = world.islands[0].as_ref().unwrap();
        for values in [
            &island.bodies,
            &world.activation_queue,
            &world.pending_bodies,
        ] {
            actual.push(values.len() as u32);
            actual.extend_from_slice(values);
        }
        actual.extend([
            u32::from(island.wake),
            u32::from(island.queued),
            world.body_commands.len() as u32,
        ]);
        for command in &world.body_commands {
            actual.extend([command.body, u32::from(command.activated)]);
        }
        let expected: Vec<_> = words.collect();
        if connected {
            assert_eq!(
                actual,
                expected[..actual.len()],
                "case {count}: connected owner state"
            );
            let mut events = &expected[actual.len() + 1..];
            assert_eq!(events.len(), expected[actual.len()] as usize);
            let mut signals = Vec::new();
            let mut marked = Vec::new();
            while !events.is_empty() {
                let n = events[2] as usize;
                match events[0] {
                    0xa0 => signals.extend_from_slice(&events[..3 + n]),
                    0xb0 => {
                        assert_eq!(events[1], 4);
                        marked.extend_from_slice(&events[3..3 + n]);
                    }
                    _ => panic!("unexpected source event"),
                }
                events = &events[3 + n..];
            }
            assert_eq!(
                callbacks.events, signals,
                "case {count}: connected signal dispatch"
            );
            let mask = marked
                .iter()
                .fold(0, |mask, id| mask | (1 << (id & 0xffffff)));
            assert_eq!(broadphase.pending_body_bits().len(), 8);
            assert_eq!(
                broadphase.pending_body_bits().words(),
                [mask],
                "case {count}: real marks"
            );
            let range = marked
                .iter()
                .map(|id| id & 0xffffff)
                .min()
                .zip(marked.iter().map(|id| id & 0xffffff).max())
                .map(|(a, b)| [a, b]);
            assert_eq!(broadphase.pending_body_range(), range);
        } else {
            actual.push(callbacks.events.len() as u32);
            actual.extend(callbacks.events);
            assert_eq!(actual, expected, "case {count}");
        }
        count += 1;
    }
    assert_eq!(count, 1024);
}
