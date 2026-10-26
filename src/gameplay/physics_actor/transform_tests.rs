use super::*;
use crate::gameplay::reference_frame::ReferenceFrame;

fn vector(words: &mut impl Iterator<Item = u32>) -> [f32; 3] {
    std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
}
fn pose(words: &mut impl Iterator<Item = u32>) -> ReferenceFrame {
    ReferenceFrame {
        position: vector(words),
        basis: std::array::from_fn(|_| vector(words)),
    }
}
fn snapshot(actor: &PhysicsActor) -> Vec<u32> {
    let mut out = vec![u32::from(actor.flags), actor.frame.marker as u32];
    for p in [actor.frame.current] {
        out.extend(p.position.map(f32::to_bits));
        out.extend(p.basis.into_iter().flatten().map(f32::to_bits));
    }
    out.extend(actor.frame.scale.map(f32::to_bits));
    out.extend(
        actor
            .frame
            .published_velocities
            .into_iter()
            .flatten()
            .map(f32::to_bits),
    );
    out.extend(actor.frame.predicted.position.map(f32::to_bits));
    out.extend(
        actor
            .frame
            .predicted
            .basis
            .into_iter()
            .flatten()
            .map(f32::to_bits),
    );
    out.extend(
        actor
            .motion
            .velocities
            .into_iter()
            .flatten()
            .map(f32::to_bits),
    );
    out.extend(actor.motion.valid.map(u32::from));
    out.extend(actor.motion.dirty.map(u32::from));
    out
}
#[test]
fn transform_notifications_match_complete_executed_actor_callback() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/actor_transform.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let clear = words.next().unwrap() != 0;
        let mut actor = PhysicsActor::new(0);
        actor.flags = words.next().unwrap() as u8;
        actor.frame.marker = words.next().unwrap() as i32;
        actor.frame.current = pose(&mut words);
        actor.frame.scale = vector(&mut words);
        actor.frame.published_velocities = std::array::from_fn(|_| vector(&mut words));
        actor.frame.predicted = pose(&mut words);
        actor.motion.velocities = std::array::from_fn(|_| vector(&mut words));
        actor.motion.valid = std::array::from_fn(|_| words.next().unwrap() != 0);
        actor.motion.dirty = std::array::from_fn(|_| words.next().unwrap() != 0);
        actor.set_transform(pose(&mut words), clear);
        assert_eq!(snapshot(&actor), words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
