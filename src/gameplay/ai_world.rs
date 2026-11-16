//! AIWorld identity/area routing (8aa288..8aa605,8a8640) and the real
//! AIDangerZone receiver (41e220,41f430/41f490/41f460).
//! Coordination Blackboard (+370) is a different component from danger (+380).
use super::entity_lifetime::{EntityId, LiveEntities};
use super::{component_state::{ComponentOwner, ComponentInstance}, pickable_component::CheckedComponent};
use std::sync::Arc;
pub mod managers;
pub mod startup_managers;
mod dsm_influence;
pub mod crew;
pub mod crew_collectors;
pub mod crew_modules;
pub mod crew_tree;

#[derive(Clone, Copy, Debug)]
pub struct SphereDanger {
    pub entity: EntityId,
    pub mode: i32,
    pub velocity: [f32; 3],
    pub position: [f32; 3],
    pub radius: f32,
}

/// 41d380 starts the 128-record in-place list empty. Host: reserve only the
/// admitted numeric rows; immutable resources and unused NPC arrays are absent.
#[derive(Default)]
pub struct DangerZones {
    spheres: Vec<SphereDanger>,
    dirty: bool,
}
impl DangerZones {
    pub fn spheres(&self) -> &[SphereDanger] { &self.spheres }
    pub fn dirty(&self) -> bool { self.dirty }
    /// 41d760 appends even when another record has the same source entity.
    pub fn add(&mut self, value: SphereDanger) { self.spheres.push(value); self.dirty = true; }
    /// 41f490/41f280 update the first matching entity without replacing mode.
    pub fn update(&mut self, value: SphereDanger) -> Result<(), String> {
        let row = self.spheres.iter_mut().find(|row| row.entity == value.entity)
            .ok_or("AI danger update has no admitted source zone")?;
        row.position = value.position;
        row.radius = value.radius;
        row.velocity = value.velocity;
        self.dirty = true;
        Ok(())
    }
    /// 41ed10 removes the first matching row by copying the last live row.
    pub fn remove(&mut self, entity: EntityId) {
        if let Some(index) = self.spheres.iter().position(|row| row.entity == entity) { self.spheres.swap_remove(index); }
        self.dirty = true;
    }
    /// 41eea0 runs before danger evaluation, releasing stale managed sources.
    pub fn retire_stale(&mut self, live: &LiveEntities) {
        for index in (0..self.spheres.len()).rev() {
            if live.get(self.spheres[index].entity).is_none() {
                self.spheres.swap_remove(index); self.dirty = true;
            }
        }
    }
}

pub struct AIWorld {
    pub entity: EntityId,
    pub bounds_owner: Option<EntityId>,
    pub bounds: Option<[[f32; 3]; 2]>,
    components: [Option<CheckedComponent>; 6],
}

/// Actual AIWorld members reflected by141592d40, not arbitrary offsets.
#[derive(Clone, Copy, Debug)]
pub enum Slot { Dsm, Coordination, Vision, Danger, Timers, Cues }
impl Slot {
    pub fn source_offset(self) -> u16 { match self {
        Self::Dsm => 0x368, Self::Coordination => 0x370, Self::Vision => 0x378,
        Self::Danger => 0x380, Self::Timers => 0x388, Self::Cues => 0x390,
    } }
}

pub struct DangerComponent {
    pub owner: ComponentOwner,
    pub definition: Arc<crate::assets::ai_world::DangerDefinition>,
    pub zones: DangerZones,
    pub initialized: bool,
    pub started: bool,
}
impl DangerComponent {
    pub fn new(definition: Arc<crate::assets::ai_world::DangerDefinition>) -> Self {
        let mut owner = ComponentOwner::default();
        owner.request_update(1); //41d380 ->7e7ab0
        Self { owner, definition, zones: DangerZones::default(), initialized: false, started: false }
    }
}
impl ComponentInstance for DangerComponent {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}

/// The game-local m_aiWorlds list owns actual spawned AIWorlds. Callers must
/// supply canonical entity identities; this owner never fabricates receivers.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub struct Worlds {
    worlds: Vec<AIWorld>,
}
impl Worlds {
    pub fn new(default_entity: EntityId) -> Self {
        Self { worlds: vec![AIWorld { entity: default_entity, bounds_owner: None,
            bounds: None, components: [None; 6] }] }
    }
    /// Only the real LoadMap section/navmesh admission branch calls this.
    pub fn add_section(&mut self, entity: EntityId, bounds_owner: EntityId, bounds: [[f32; 3]; 2]) -> usize {
        let index = self.worlds.len();
        self.worlds.push(AIWorld { entity, bounds_owner: Some(bounds_owner), bounds: Some(bounds), components: [None; 6] });
        index
    }
    pub fn entries(&self) -> &[AIWorld] { &self.worlds }
    /// 8a8640 tests world1..N in order, with inclusive bounds, then world0.
    pub fn area_at(&self, position: [f32; 3]) -> usize {
        self.worlds.iter().enumerate().skip(1).find_map(|(index, world)| {
            world.bounds.filter(|bounds| (0..3).all(|axis|
                bounds[0][axis] <= position[axis] && position[axis] <= bounds[1][axis])).map(|_| index)
        }).unwrap_or(0)
    }
    pub fn assign(&self, live: &mut LiveEntities, entity: EntityId, position: [f32; 3]) -> Result<(), String> {
        let area = self.area_at(position);
        self.assign_area(live, entity, area)
    }
    pub fn assign_area(&self, live: &mut LiveEntities, entity: EntityId, area: usize) -> Result<(), String> {
        if area >= self.worlds.len() { return Err("entity AIWorld index is outside the admitted world list".into()); }
        live.set_ai_world_index(entity, area as u32)
    }
    ///87d200's explicit SetTransform branch resolves8a8640 then calls the
    ///875fc0 notifier. Ordinary753c10 physics-frame copies do not call this.
    pub fn assign_transformed(&self,live:&mut LiveEntities,events:&mut super::entity_events::EntityEvents,
        entity:EntityId,position:[f32;3])->Result<bool,String>{
        let area=self.area_at(position);
        if area>=self.worlds.len(){return Err("SetTransform AIWorld owner absent".into())}
        live.change_ai_world_index(entity,area as u32,events)
    }
    pub fn recipient(&self, live: &LiveEntities, entity: EntityId) -> Result<EntityId, String> {
        let area = live.ai_world_index(entity)? as usize;
        self.worlds.get(area).map(|world| world.entity).ok_or_else(|| "assigned AIWorld is absent".into())
    }
    pub fn danger(&self, recipient: EntityId) -> Result<Option<CheckedComponent>, String> {
        self.component(recipient, Slot::Danger)
    }
    pub fn set_danger(&mut self, recipient: EntityId, danger: Option<CheckedComponent>) -> Result<(), String> {
        self.worlds.iter_mut().find(|world| world.entity == recipient).ok_or("AIWorld danger parent is absent")?.components[Slot::Danger as usize] = danger;
        Ok(())
    }
    pub fn component(&self, recipient: EntityId, slot: Slot) -> Result<Option<CheckedComponent>, String> {
        self.worlds.iter().find(|world| world.entity == recipient).map(|world| world.components[slot as usize])
            .ok_or_else(|| "component parent is not an admitted AIWorld".into())
    }
    pub fn bind_component(&mut self, recipient: EntityId, slot: Slot, checked: CheckedComponent) -> Result<(), String> {
        self.worlds.iter_mut().find(|world| world.entity == recipient).ok_or("AIWorld component parent is absent")?.components[slot as usize] = Some(checked);
        Ok(())
    }
    pub fn unbind_component(&mut self, recipient: EntityId, slot: Slot, checked: CheckedComponent) -> Result<(), String> {
        let value = &mut self.worlds.iter_mut().find(|world| world.entity == recipient).ok_or("AIWorld component parent is absent")?.components[slot as usize];
        if value.is_some_and(|actual| actual.handle != checked.handle || actual.generation != checked.generation) {
            return Err("AIWorld component binding was replaced before deinitialization".into());
        }
        *value = None; Ok(())
    }
}
