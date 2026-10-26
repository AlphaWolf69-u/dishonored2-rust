use super::*;

#[test]
fn construction_matches_complete_executed_actor_constructor() {
    for line in include_str!("../../../tests/fixtures/actor_construction.txt").lines() {
        let actor = PhysicsActor::new(91);
        let frames = &actor.frame;
        assert!(actor.owner.entity.is_none() && actor.owner.declaration.is_none());
        assert_eq!(actor.current_body, actor.default_body);
        assert_eq!(actor.default_body, 91);
        assert!(actor.parent.is_none() && actor.children.is_empty());
        // Unattached base-component flags are zero; runtime owner admission
        // sets them separately from the actor's own +408 scheduling flags.
        let mut actual = vec![
            actor.owner.generation,
            actor.owner.flags,
            actor.last_queued_frame as u32,
            actor.last_post_frame as u32,
            u32::from(actor.flags),
        ];
        for requests in &actor.requests {
            actual.extend([
                requests.timed_count as u32,
                requests.next_id,
                requests.cached_id,
                u32::from(requests.enabled),
                requests.entries().len() as u32,
            ]);
        }
        actual.extend(
            actor
                .motion
                .velocities
                .into_iter()
                .flatten()
                .map(f32::to_bits),
        );
        actual.extend(actor.motion.valid.map(u32::from));
        actual.extend(actor.motion.dirty.map(u32::from));
        actual.extend(frames.current.position.map(f32::to_bits));
        actual.extend(frames.current.basis.into_iter().flatten().map(f32::to_bits));
        actual.extend(frames.scale.map(f32::to_bits));
        actual.extend(
            frames
                .published_velocities
                .into_iter()
                .flatten()
                .map(f32::to_bits),
        );
        actual.extend(frames.predicted.position.map(f32::to_bits));
        actual.extend(
            frames
                .predicted
                .basis
                .into_iter()
                .flatten()
                .map(f32::to_bits),
        );
        actual.push(frames.marker as u32);
        assert!(actor.binding.entity.is_none());
        actual.push(actor.binding.joint as i32 as u32);
        actual.extend(actor.binding.offset.map(f32::to_bits));
        actual.extend(actor.binding.orientation.map(f32::to_bits));
        actual.push(u32::from(actor.binding.phase));
        let expected: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        assert_eq!(actual, expected);
    }
}
