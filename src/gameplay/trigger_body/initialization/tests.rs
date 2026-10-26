use super::*;

struct World {
    trigger: TriggerComponent,
    collision: TriggerCollisionSettings,
    water: bool,
    wind: bool,
    entity_flags: u8,
    material: u16,
    flip: bool,
    missing: bool,
    body: u32,
    trace: Vec<u32>,
}
impl World {
    fn snapshot(&mut self, event: u32) {
        self.trace.extend([
            event,
            self.trigger.owner.flags,
            u32::from(self.trigger.spawn_owner_bit30),
            u32::from(self.trigger.water.is_some()),
            u32::from(self.trigger.wind.is_some()),
            u32::from(self.entity_flags),
            self.body,
        ]);
    }
}
impl TriggerInitializationWorld for World {
    fn trigger(&self, _: ComponentHandle) -> Result<&TriggerComponent, String> {
        Ok(&self.trigger)
    }
    fn trigger_mut(&mut self, _: ComponentHandle) -> Result<&mut TriggerComponent, String> {
        Ok(&mut self.trigger)
    }
    fn load_shape(&mut self, entity: usize) -> Result<Option<LoadedTriggerShape>, String> {
        assert_eq!(entity, 11);
        self.snapshot(1);
        Ok((!self.missing).then_some(LoadedTriggerShape {
            shape: 23,
            basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        }))
    }
    fn report_missing_shape(&mut self, entity: usize) -> Result<(), String> {
        assert_eq!(entity, 11);
        self.snapshot(6);
        Ok(())
    }
    fn release_failed(&mut self, _: ComponentHandle) -> Result<(), String> {
        self.snapshot(7);
        Ok(())
    }
    fn dependency(
        &mut self,
        entity: usize,
        kind: TriggerDependency,
    ) -> Result<Option<ComponentHandle>, String> {
        assert_eq!(entity, 11);
        let present = match kind {
            TriggerDependency::Water => {
                self.snapshot(2);
                self.water
            }
            TriggerDependency::Wind => {
                self.snapshot(3);
                self.wind
            }
        };
        Ok(present.then_some(ComponentHandle {
            class_id: 31,
            slot: 9,
        }))
    }
    fn position(&self, entity: usize) -> Result<[f32; 3], String> {
        assert_eq!(entity, 11);
        Ok([1., 2., 3.])
    }
    fn declaration(&self, _: ComponentHandle) -> Result<usize, String> {
        Ok(17)
    }
    fn collision_settings(&self, declaration: usize) -> Result<TriggerCollisionSettings, String> {
        assert_eq!(declaration, 17);
        Ok(self.collision)
    }
    fn trigger_material(&self) -> Result<u16, String> {
        Ok(self.material)
    }
    fn create_disabled_body(
        &mut self,
        reference: ComponentHandle,
        info: &BodyInfo,
    ) -> Result<u32, String> {
        self.snapshot(4);
        assert_eq!(info.shape, 23);
        assert_eq!(
            info.user_data,
            CollisionUserData {
                component: reference
            }
            .encode()
        );
        assert_eq!(info.position, [1., 2., 3., 0.]);
        assert_eq!(info.material_id, self.material);
        assert_eq!(info.motion_type, u8::from(self.collision.keyframed));
        assert_eq!(info.flags, 0x10);
        if self.flip {
            self.collision.keyframed = !self.collision.keyframed;
        }
        Ok(0x1234)
    }
    fn set_body(&mut self, _: ComponentHandle, body: u32) -> Result<(), String> {
        self.body = body;
        Ok(())
    }
    fn mark_entity_keyframed(&mut self, entity: usize) -> Result<(), String> {
        assert_eq!(entity, 11);
        self.entity_flags |= 4;
        Ok(())
    }
}

#[test]
fn initialization_matches_executed_owner_state_and_callback_sequence() {
    let mut cases = 0;
    for row in include_str!("../../../../tests/fixtures/trigger_initialization.txt").lines() {
        let words: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let flag = |bit: u32| words[0] >> bit & 1u32 != 0;
        let mut trigger = TriggerComponent::new(&mut Vec::new()).unwrap();
        trigger.owner.entity = Some(11);
        trigger.owner.flags = words[1];
        let mut world = World {
            trigger,
            collision: TriggerCollisionSettings {
                debris: flag(0),
                rigid_bodies: flag(1),
                npc_ragdolls: flag(2),
                npc_capsules: flag(3),
                keyframed: flag(4),
            },
            water: flag(5),
            wind: flag(6),
            entity_flags: words[2] as u8,
            material: words[3] as u16,
            flip: words[4] != 0,
            missing: words[5] != 0,
            body: 0x00ff_ffff,
            trace: Vec::new(),
        };
        initialize_trigger(
            ComponentHandle {
                class_id: 5,
                slot: 1,
            },
            &mut world,
        )
        .unwrap();
        world.snapshot(5);
        assert_eq!(world.trace, &words[6..], "case{cases}");
        cases += 1;
    }
    assert_eq!(cases, 640);
}
