use super::super::tests::{read_world, snapshot};
use super::*;

#[test]
fn notifications_and_detachment_match_executed_routine_with_callbacks() {
    let mut cases = 0;
    for row in include_str!("../../../tests/fixtures/deactivation_notifications.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let phase = words.next().unwrap();
        let deactivated = words.next().unwrap() != 0;
        let suppress = words.next().unwrap() != 0;
        let callback = words.next().unwrap();
        let mut bodies = BodyStorage::new(5).unwrap();
        let mut ids = [0; 5];
        for (i, id) in ids.iter_mut().enumerate() {
            let reserved = if i == 0 {
                0
            } else {
                bodies.reserve(&mut []).unwrap()
            };
            let body = bodies.get_mut(reserved).unwrap();
            body.id = words.next().unwrap();
            *id = body.id;
            body.motion_id = words.next().unwrap();
            body.next_attached_body_id = words.next().unwrap();
            body.broadphase_id = words.next().unwrap();
            body.flags = words.next().unwrap();
        }
        let mut world = read_world(&mut words);
        world.suppress_island_detach = suppress;
        let mut events = Vec::new();
        world
            .notify_activation(
                &mut bodies,
                ids[1],
                deactivated,
                phase,
                |world, bodies, members, activated| {
                    events.extend([u32::from(activated), members.len() as u32]);
                    events.extend_from_slice(members);
                    if callback & 1 != 0 {
                        bodies.get_mut(ids[1]).unwrap().next_attached_body_id = ids[3];
                    }
                    if callback & 2 != 0 {
                        world.state_mut(1).unwrap().set_island(1);
                    }
                    if callback & 4 != 0 {
                        world.suppress_island_detach = !world.suppress_island_detach;
                    }
                    Ok(())
                },
            )
            .unwrap();
        let expected = read_world(&mut words);
        assert_eq!(
            snapshot(&world),
            snapshot(&expected),
            "case {cases}: island state"
        );
        let mut actual = vec![
            u32::from(world.suppress_island_detach),
            world.body_commands.len() as u32,
        ];
        for command in &world.body_commands {
            actual.extend([command.body, u32::from(command.activated)]);
        }
        actual.push(events.len() as u32);
        actual.extend(events);
        assert_eq!(
            actual,
            words.collect::<Vec<_>>(),
            "case {cases}: commands/signals"
        );
        cases += 1;
    }
    assert_eq!(cases, 768);
}
