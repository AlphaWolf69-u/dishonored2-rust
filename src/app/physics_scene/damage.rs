//! Base world health on actual admitted entity/component identities.
//! 717e70/7181c0 startup, 717800 calculation, 717950/717fe0/878d20 death.
use super::*;
use crate::gameplay::{component_state::ComponentInstance, damage::{Context, DamageResult, Parameters, Receiver}, entity_lifetime::{EntityId, LiveEntities, Stage}};

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
    /// Reflected damageTaken/incapacitated script callbacks require their
    /// script receiver; numeric EntityEvents IDs are a different namespace.
    pub(in crate::app) fn damage_entity(&mut self, live: &mut LiveEntities, id: EntityId,
        params: &Parameters) -> Result<Option<DamageResult>, String> {
        let Some(handle) = self.health_component(live, id)? else { return Ok(None) };
        let index = self.owner(handle)?.entity.ok_or("health target is unlinked")?;
        if self.entities[index].entity_flags & 0x800 != 0 { return Ok(Some(DamageResult::default())); }
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
                health.attacker = params.attacker.or(params.inflictor);
                self.entities[index].entity_flags = (self.entities[index].entity_flags & !0x2000) | 0x800;
                live.set_requested_entity_context(id, 2)?;
            }
        }
        Ok(Some(result))
    }
    /// Reuse collision user data's canonical component/entity resolution.
    pub(in crate::app) fn damage_collision_target(&mut self, live: &mut LiveEntities, user_data: u64,
        params: &Parameters) -> Result<Option<DamageResult>, String> {
        if user_data == 0 { return Ok(None); }
        let index = crate::gameplay::collision_user_data::CollisionUserData::decode(user_data)?.entity(|handle| self.owner(handle).ok())?;
        let id = self.entities.get(index).ok_or("damage collider entity absent")?.shared;
        self.damage_entity(live, id, params)
    }
}
