use super::*;
use crate::gameplay::{motion_info::MotionInfo, space_cells::SpaceCells};

fn list(words: &mut impl Iterator<Item = u32>) -> Vec<u32> {
    let count = words.next().unwrap() as usize;
    words.take(count).collect()
}

pub(crate) fn snapshot(bodies: &BodyStorage, motions: &MotionStorage) -> Vec<u32> {
    let mut values = Vec::new();
    for i in 1..8 {
        let body = bodies.get_by_index(i).unwrap();
        values.extend([body.flags, u32::from(body.status), body.active_list_index]);
    }
    values.push(bodies.active_body_ids().len() as u32);
    values.extend_from_slice(bodies.active_body_ids());
    values.extend(bodies.allocation_marks().iter().map(|&v| u32::from(v)));
    values.extend((1..4).map(|i| motions.get(i).unwrap().solver_id));
    for cell in motions.cell_lists() {
        values.push(cell.len() as u32);
        values.extend_from_slice(cell);
    }
    values.push(bodies.changes().len() as u32);
    for change in bodies.changes() {
        values.extend([
            change.body_index,
            change.queue_index,
            u32::from(change.flags),
        ]);
    }
    for activation in [BodyActivation::Activate, BodyActivation::KeepDeactivated] {
        let queue = bodies.queued_additions(activation);
        values.push(queue.len() as u32);
        values.extend_from_slice(queue);
    }
    values.extend_from_slice(bodies.change_indices());
    values
}

pub(crate) fn initialize(words: &mut impl Iterator<Item = u32>) -> (BodyStorage, MotionStorage) {
    let mut bodies = BodyStorage::new(8).unwrap();
    let mut motions = MotionStorage::new(4).unwrap();
    let cells: [u8; 3] = std::array::from_fn(|_| words.next().unwrap() as u8);
    for cell in cells {
        let id = motions.reserve().unwrap();
        motions
            .initialize(id, &MotionInfo::default(), &SpaceCells::Single)
            .unwrap();
        motions.get_mut(id).unwrap().cell = cell;
    }
    for _ in 1..8 {
        let id = bodies.reserve(&mut []).unwrap();
        let body = bodies.get_mut(id).unwrap();
        body.id = words.next().unwrap();
        body.motion_id = words.next().unwrap();
        body.flags = words.next().unwrap();
        body.status = words.next().unwrap() as u16;
        body.next_attached_body_id = words.next().unwrap();
        body.broadphase_id = words.next().unwrap();
        body.active_list_index = words.next().unwrap();
    }
    for activation in [BodyActivation::Activate, BodyActivation::KeepDeactivated] {
        bodies.queue_additions(&list(words), activation).unwrap();
    }
    (bodies, motions)
}

#[test]
fn active_body_and_motion_lists_match_executed_ring_and_queue_transitions() {
    let mut bodies = BodyStorage::new(8).unwrap();
    let mut motions = MotionStorage::new(4).unwrap();
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/body_activation_lists.txt").lines() {
        let mut words = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        if words.next().unwrap() != 0 {
            (bodies, motions) = initialize(&mut words);
        }
        let operation = words.next().unwrap();
        let target = words.next().unwrap();
        match operation {
            0 => bodies.activate_attached(target, &mut motions).unwrap(),
            1 => bodies.remove_active(target).unwrap(),
            2 => motions.remove_from_solver(target).unwrap(),
            3 => motions.add_to_solver(target).unwrap(),
            _ => panic!("unknown native operation"),
        }
        assert_eq!(
            snapshot(&bodies, &motions),
            words.collect::<Vec<_>>(),
            "case {count}"
        );
        count += 1;
    }
    assert_eq!(count, 1024);
}
