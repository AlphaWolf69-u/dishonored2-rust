//! Base world health on actual admitted entity/component identities.
//! 717e70/7181c0 startup, 717800 calculation, 717950/717fe0/878d20 death.
use super::*;
use crate::gameplay::{component_state::ComponentInstance, damage::{Context, DamageResult, Parameters, Receiver}, entity_lifetime::{EntityId, LiveEntities, Stage}, entity_events::{EntityEvents, Event, Payload}};

pub(super) struct HealthComponent {
    pub owner: ComponentOwner,
    pub receiver: Receiver,
    pub maximum: f32,
    pub current: f32,
    pub attacker: Option<EntityId>,
}
impl ComponentInstance for HealthComponent {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}
impl PhysicsScene {
    /// A source may name only a real spawned entity with an admitted health
    /// component. Authored breakable tags and render placements do not create
    /// an entity/component identity or fabricate a damage receiver.
    pub(in crate::app) fn named_damage_target(&self, live: &LiveEntities, name: &str) -> Result<Option<EntityId>, String> {
        let Some(id) = live.find(name) else { return Ok(None) };
        Ok(self.health_component(live, id)?.map(|_| id))
    }
    pub(super) fn admit_health_components(&mut self, catalog: &archive::Catalog,
        document: &entities::Document, registry: &mut ComponentRegistry) -> Result<(), String> {
        let bindings = entities::damage::health_bindings(document, |path| crate::assets::declaration_texts::read_common_or_catalog(catalog, path))?;
        for (source, binding) in bindings.into_iter().enumerate() {
            // An actual actor owner admits its components. Never allocate an
            // NPC or door header as a substitute for their missing behavior.
            if binding.class != "cpnthealth" { continue; }
            let Some(index) = self.entities.iter().position(|entity| entity.name == binding.entity_id) else { continue };
            let declaration = self.types.declaration(self.factories[10], source, binding.scheduling)?;
            let handle = ComponentFactory { declaration: &declaration, pool: &mut self.health, registry, grow_one_at_a_time: false }
                .create(&mut self.entities[index].components, declaration.scheduling.starts_on_spawn(),
                    |_, _, _| Ok(HealthComponent { owner: Default::default(), receiver: binding.receiver,
                        maximum: binding.maximum, current: 1.0, attacker: None }), |_, _, _| Ok(()))?
                .ok_or("health component creation disabled")?;
            self.declarations.insert(handle.packed(), declaration);
        }
        Ok(())
    }
    /// Query the actual base health component through the entity's own list.
    /// A collider without this component never becomes damageable implicitly.
    pub(in crate::app) fn entity_health(&self, live: &LiveEntities, id: EntityId) -> Result<Option<(f32, f32)>, String> {
        let Some(handle) = self.health_component(live, id)? else { return Ok(None) };
        let health = self.health.get(handle.slot).ok_or("health payload absent")?;
        Ok(Some((health.current, health.maximum)))
    }
    fn health_component(&self, live: &LiveEntities, id: EntityId) -> Result<Option<ComponentHandle>, String> {
        if live.stage(id) != Some(Stage::Spawned) { return Err("damage target identity is stale or not spawned".into()); }
        let Some(&index) = self.shared_indices.get(&id) else { return Ok(None) };
        self.entities[index].components.find_component([self.classes[10]; 2], [0x20, 0], |handle| {
            Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState { flags: self.owner(handle)?.flags,
                start_phases: self.declarations.get(&handle.packed()).map(|declaration| declaration.scheduling.start_phases) })
        })
    }
    /// 717950 consumes a source's authored parameters and changes the same
    /// live entity flags/context used by normal component lifecycle updates.
    /// 876660/8767a0/8769f0 bridge the reflected events into the actual
    /// numeric entity message queue, retaining their source payloads.
    pub(in crate::app) fn damage_entity(&mut self, live: &mut LiveEntities, events: &mut EntityEvents, id: EntityId,
        params: &Parameters) -> Result<Option<DamageResult>, String> {
        let Some(handle) = self.health_component(live, id)? else { return Ok(None) };
        let index = self.owner(handle)?.entity.ok_or("health target is unlinked")?;
        if self.entities[index].entity_flags & 0x800 != 0 {
            events.post(Event { id: 0x1a6, target: Some(id), payload: Payload::DamageParameters(*params) });
            return Ok(Some(DamageResult::default()));
        }
        let context = Context { forward: bevy_math::Vec3::from_array(self.entities[index].frame.pose.basis[0]),
            started: self.owner(handle)?.flags & 0xc >= 8, ..Default::default() };
        let health = self.health.get_mut(handle.slot).ok_or("health payload absent")?;
        let result = health.receiver.evaluate(health.current, params, context);
        if result.accepted {
            if !health.receiver.infinite_health { health.current = (health.current - result.health_damage).clamp(0.0, health.maximum); }
            if result.incapacitated {
                // +d1 bit3 is stored in entity_flags bit11 here. 878d20
                // requests DEAD(2), using the live context queue (87ce50).
                health.current = 0.0;
                health.attacker = params.attacker;
                self.entities[index].entity_flags = (self.entities[index].entity_flags & !0x2000) | 0x800;
                live.set_requested_entity_context(id, 2)?;
                events.post(Event { id: 0xa5, target: Some(id), payload: Payload::DamageParameters(*params) });
            }
        }
        // 717950 checks the type filter again before the base sleep
        // callback718110. A rejected direction alone does not suppress it.
        if !result.incapacitated && health.receiver.accepts(params.damage_type)
            && params.damage_type.intersects(crate::gameplay::damage::DamageType::SLEEP)
            && self.entities[index].entity_flags & 0x2800 == 0 {
            live.set_requested_entity_context(id, 1)?;
            self.entities[index].entity_flags |= 0x2000;
            health.attacker = params.attacker;
            events.post(Event { id: 0xa5, target: Some(id), payload: Payload::DamageParameters(*params) });
        }
        events.post(Event { id: 0xa2, target: Some(id), payload: Payload::Damage { params: *params, result } });
        if let Some(attacker) = params.attacker.filter(|attacker| live.get(*attacker).is_some()) {
            events.post(Event { id: 0xa3, target: Some(attacker), payload: Payload::Damaging { target: id, params: *params, result } });
        }
        Ok(Some(result))
    }
    /// Reuse collision user data's canonical component/entity resolution.
    pub(in crate::app) fn damage_collision_target(&mut self, live: &mut LiveEntities, events: &mut EntityEvents, user_data: u64,
        params: &Parameters) -> Result<Option<DamageResult>, String> {
        if user_data == 0 { return Ok(None); }
        let index = crate::gameplay::collision_user_data::CollisionUserData::decode(user_data)?.entity(|handle| self.owner(handle).ok())?;
        let id = self.entities.get(index).ok_or("damage collider entity absent")?.shared;
        self.damage_entity(live, events, id, params)
    }
}
