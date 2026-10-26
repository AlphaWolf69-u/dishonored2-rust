use super::*;
use crate::gameplay::physics_actor::{ActorBody, PhysicsActor, PhysicsActorLinks};

#[test]
fn collision_component_ownership_matches_trigger_setup_and_actor_attachment() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/trigger_data_ownership.txt").lines() {
        let words: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let handle = ComponentHandle {
            class_id: 17,
            slot: 0,
        };
        let owner = ComponentOwner {
            entity: Some(42 + words[0] as usize),
            ..Default::default()
        };
        let encoded = CollisionUserData { component: handle }.encode();
        let mut links = PhysicsActorLinks {
            actors: crate::gameplay::physics_actor::test_actor_pool(vec![PhysicsActor::new(0)]),
            bodies: vec![
                ActorBody {
                    actor: None,
                    body_id: u32::MAX,
                },
                ActorBody {
                    actor: None,
                    body_id: 0x1234,
                },
            ],
        };
        links.actors[0].owner.entity = owner.entity;
        let state = |links: &PhysicsActorLinks| {
            let data = CollisionUserData::decode(encoded).unwrap();
            let entity = data
                .entity(|key| (key == handle).then_some(&owner))
                .unwrap();
            [
                if data.component == handle { 1 } else { 2 },
                entity as u32,
                if links.bodies[1].actor.is_some() {
                    2
                } else {
                    0
                },
            ]
        };
        let mut actual = state(&links).to_vec();
        links
            .attach_body(0, 1, 0, |_, _| {
                Err("empty request managers must not enqueue".into())
            })
            .unwrap();
        actual.extend(state(&links));
        actual.extend([
            u32::from(links.actors[0].flags),
            u32::from(links.actors[0].current_body == 1),
        ]);
        assert_eq!(actual, words[1..]);
        cases += 1;
    }
    assert_eq!(cases, 32);
}
