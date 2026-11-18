//! Live entity identities shared by component and graph lifecycle owners.
//! Parsing a map declaration alone does not insert an entity here. The engine
//! registers a successfully allocated entity's handle and name before the
//! batch initialization and spawn passes (1408ae2ba–1408ae335).
mod observers;
use hashbrown::{Equivalent, HashMap};
use super::name_pool::NameId;
use std::{hash::{Hash, Hasher}, sync::{Arc, atomic::{AtomicU64, Ordering}}};

// A level replacement must not reuse identities still held by queued work.
static NEXT_ENTITY_GENERATION: AtomicU64 = AtomicU64::new(1);

// Entity-name lookup uses the engine's ASCII-insensitive comparison
// (1408a7d90 / 140143650). Hash both owned and borrowed keys identically,
// without allocating a lowercase copy for each lookup.
struct Name(Arc<str>);
struct NameQuery<'a>(&'a str);
fn hash_name<H: Hasher>(value: &str, state: &mut H) {
    value.len().hash(state);
    for byte in value.bytes() { byte.to_ascii_lowercase().hash(state); }
}
// 140143650: compare signed bytes after folding only ASCII uppercase.
fn tag_compare(left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    let mut index = 0;
    loop {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        let order = (a.to_ascii_lowercase() as i8).cmp(&(b.to_ascii_lowercase() as i8));
        if order != std::cmp::Ordering::Equal || a == 0 { return order; }
        index += 1;
    }
}
impl Hash for Name {
    fn hash<H: Hasher>(&self, state: &mut H) { hash_name(&self.0, state); }
}
impl PartialEq for Name {
    fn eq(&self, other: &Self) -> bool { self.0.eq_ignore_ascii_case(&other.0) }
}
impl Eq for Name {}
impl Hash for NameQuery<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) { hash_name(self.0, state); }
}
impl Equivalent<Name> for NameQuery<'_> {
    fn equivalent(&self, name: &Name) -> bool { self.0.eq_ignore_ascii_case(&name.0) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityId(u64);

/// The map's actual constructed worldspawn identity for global game services.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
#[derive(Clone, Copy, Debug)]
pub struct WorldspawnEntity(pub EntityId);

struct Entry {
    name: Arc<str>,
    class: usize,
    ///idEntity+2e0, constructor874c50 initializes the actual AIWorld index0.
    ai_world_index: u32,
    stage: Stage,
    spawn_tag: NameId,
    tag_group: Option<u32>,
    // idEntity constructor 140874c50 initializes the child list at +0xb0
    // empty. The attachment owner alone may populate it after its callbacks
    // and parent binding are implemented; authored attachments remain gated.
    children: Vec<EntityId>,
    parent: Option<EntityId>,
    /// idEntity+ac is a managed owner reference, independent of attachments.
    owner: Option<EntityId>,
    ///idEventReceiver+8/+10, constructed empty by883740. Generic notice
    ///subscriptions own only generation-bearing identities, never resources.
    event_observers: Option<Box<Vec<EntityId>>>,
    event_subjects: Option<Box<Vec<EntityId>>>,
    ///Host: derived arkTrigger+360 state is allocated only for a touched
    ///native Trigger, rather than adding a Vec to every idEntity header.
    trigger_touching: Option<Box<Vec<EntityId>>>,
    spawned_index: Option<usize>,
    removal_requested: bool,
    removal_queued: bool,
    release_prepared: bool,
    context: EntityContext,
    update_mask: u32,
    update_active: bool,
}

/// idEntity +288/+28c/+290. Constructor 874c50 initializes all three to
/// ALIVE (0); requested and current remain distinct until the game-local
/// context batch commits them through 878fb0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EntityContext {
    pub current: u32,
    pub requested: u32,
    pub previous: u32,
}
/// Constructor state before an idEntity receives a managed identity. Once
/// registered it moves into the one corresponding LiveEntities entry.
#[derive(Default)]
pub(crate) struct UnregisteredEntityState {
    context: EntityContext,
    update_mask: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage { Allocated, Initialized, Spawned }
#[derive(Default)]
struct Slot {
    generation: u32,
    // Native managed indices are sparse. Keep the large entity record only
    // for occupied slots instead of expanding it across the entire range.
    entry: Option<Box<Entry>>,
    reserved: bool,
}

struct TagGroup {
    spelling: Arc<str>,
    members: Vec<EntityId>,
    next: u32,
}

#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub struct LiveEntities {
    slots: Vec<Slot>,
    // 1407b5e30 uses one cursor for both native managed-slot ranges.
    managed_cursor: usize,
    by_name: HashMap<Name, EntityId>,
    // 14065afa0 retains exact-case names for entities whose +0xd4 bit 1 is set.
    removed_names: Vec<Arc<str>>,
    // 1408a6180 records actual spawned entities in game-local +0x601b8.
    spawned_order: Vec<EntityId>,
    tag_heads: [u32; 256],
    tag_groups: Vec<TagGroup>,
    // Game-local +0x601d0: remove-event requests in admission order.
    pending_removals: Vec<EntityId>,
    active_removal_batch: Option<super::entity_release::EntityReleaseBatch>,
    removal_batch_in_flight: bool,
    // 8a62e0 / 8ab380: source order and dedup are retained while callbacks
    // run. None is the actual invalid native managed handle, not an entity.
    pending_contexts: Vec<Option<EntityId>>,
    context_cursor: usize,
    context_touched: Vec<EntityId>,
    context_commit_cursor: usize,
    context_notify_cursor: usize,
    pending_entity_updates: Vec<Option<EntityId>>,
    active_entity_updates: Vec<Option<EntityId>>,
    pending_update_removals: i32,
}

impl Default for LiveEntities {
    fn default() -> Self {
        Self { slots: Vec::new(), managed_cursor: 0, by_name: HashMap::new(),
            removed_names: Vec::new(), spawned_order: Vec::new(),
            tag_heads: [u32::MAX; 256], tag_groups: Vec::new(), pending_removals: Vec::new(),
            active_removal_batch: None, removal_batch_in_flight: false,
            pending_contexts: Vec::new(), context_cursor: 0, context_touched: Vec::new(), context_commit_cursor: 0, context_notify_cursor: 0,
            pending_entity_updates: Vec::new(), active_entity_updates: Vec::new(), pending_update_removals: 0 }
    }
}

impl LiveEntities {
    /// Register a constructed entity in the native managed container. The
    /// optional explicit index is used by the real caller that selects it;
    /// otherwise the entity's actual +0xd4 bit 3 chooses the allocator range.
    pub fn allocate_managed(&mut self, name: Arc<str>, class: usize, requested: Option<u16>, runtime_flags: u8, constructor_spawn_tag: NameId) -> Result<EntityId, String> {
        static REPORT_LAYOUT: std::sync::Once = std::sync::Once::new();
        REPORT_LAYOUT.call_once(|| {
            if std::env::var_os("DH2_DEBUG_ENTITY_RESOURCES").is_some() {
                eprintln!("entity managed layout: slot={} entry={} bytes (excluding heap buffers)",
                    std::mem::size_of::<Slot>(), std::mem::size_of::<Entry>());
            }
        });
        if self.by_name.contains_key(&NameQuery(&name)) { return Err(format!("entity already live: {name}")); }
        const LIMIT: usize = 0x4000;
        let index = if let Some(index) = requested {
            let index = usize::from(index);
            if index >= LIMIT { return Err("managed entity slot out of range".into()); }
            if self.slots.get(index).is_some_and(|slot| slot.entry.is_some() || slot.reserved) {
                return Err("managed entity slot is occupied or reserved".into());
            }
            index
        } else {
            let (start, end) = if runtime_flags & 8 != 0 { (0, 1000) } else { (1000, LIMIT) };
            if self.managed_cursor < start || self.managed_cursor >= end { self.managed_cursor = start; }
            let first = self.managed_cursor;
            (first..end).chain(start..first)
                .find(|&index| self.slots.get(index).is_none_or(|slot| slot.entry.is_none() && !slot.reserved))
                .ok_or("managed entity slot range exhausted")?
        };
        if self.slots.len() <= index {
            self.slots.try_reserve(index + 1 - self.slots.len()).map_err(|error| format!("managed entity slots: {error}"))?;
            self.slots.resize_with(index + 1, Slot::default);
        }
        let generation = u32::try_from(NEXT_ENTITY_GENERATION.fetch_add(1, Ordering::Relaxed))
            .map_err(|_| "entity generation range exhausted")?;
        let slot = &mut self.slots[index];
        slot.generation = generation;
        let id = EntityId((u64::from(slot.generation) << 32) | index as u64);
        slot.entry = Some(Box::new(Entry { name: name.clone(), class, ai_world_index: 0, stage: Stage::Allocated, spawn_tag: constructor_spawn_tag, tag_group: None,
            children: Vec::new(), parent: None, owner: None, event_observers: None, event_subjects: None, trigger_touching: None, spawned_index: None, removal_requested: false, removal_queued: false,
            release_prepared: false, context: EntityContext::default(), update_mask: 0, update_active: false }));
        self.by_name.insert(Name(name), id);
        if requested.is_none() { self.managed_cursor = index; }
        if index == self.managed_cursor { self.managed_cursor += 1; }
        Ok(id)
    }

    /// 657ae0 allocates the real managed slot before 87b780 assigns an
    /// otherwise unnamed entity. Its suffix is the signed slot halfword, not
    /// the host's generation; occupied names advance the suffix independently.
    /// Compute the next name without publishing an identity. The exclusive
    /// factory must register immediately after reflection and revalidate it.
    pub(crate) fn preview_auto_name(&self, declaration: &str, runtime_flags: u8) -> Result<(u16, Arc<str>), String> {
        let (start, end) = if runtime_flags & 8 != 0 { (0, 1000) } else { (1000, 0x4000) };
        let cursor = if self.managed_cursor < start || self.managed_cursor >= end { start } else { self.managed_cursor };
        let index = (cursor..end).chain(start..cursor)
            .find(|&index| self.slots.get(index).is_none_or(|slot| slot.entry.is_none() && !slot.reserved))
            .ok_or("managed entity slot range exhausted")?;
        let last_separator = declaration.rfind(['/', '\\']);
        let last_dot = declaration.rfind('.').filter(|dot| last_separator.is_none_or(|separator| *dot > separator));
        let stem = &declaration[..last_dot.unwrap_or(declaration.len())];
        let base = stem.rsplit(['/', '\\']).next().ok_or("entity declaration name is absent")?;
        if base.is_empty() { return Err("entity declaration has an empty auto-name base".into()); }
        let mut suffix = i32::from(index as i16);
        let name = loop {
            let candidate = format!("{base}_{suffix}");
            if self.find(&candidate).is_none() { break Arc::<str>::from(candidate); }
            suffix = suffix.checked_add(1).ok_or("entity auto-name suffix overflow")?;
        };
        Ok((index as u16, name))
    }

    /// 657ae0 allocates the real managed slot before 87b780 assigns its
    /// generated name. The exclusive operation uses that same slot and name.
    pub fn allocate_managed_auto_name(&mut self, declaration: &str, class: usize,
        runtime_flags: u8, constructor_spawn_tag: NameId) -> Result<EntityId, String> {
        let (index, name) = self.preview_auto_name(declaration, runtime_flags)?;
        let id = self.allocate_managed(name, class, Some(index), runtime_flags, constructor_spawn_tag)?;
        self.managed_cursor = usize::from(index) + 1;
        Ok(id)
    }

    fn advance(&mut self, id: EntityId, from: Stage, to: Stage) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_mut().ok_or("entity already retired")?;
        if entry.stage != from { return Err(format!("entity {} is {:?}, expected {from:?}", entry.name, entry.stage)); }
        entry.stage = to;
        Ok(())
    }
    pub fn initialized(&mut self, id: EntityId) -> Result<(), String> {
        self.advance(id, Stage::Allocated, Stage::Initialized)
    }
    pub fn spawned(&mut self, id: EntityId) -> Result<(), String> {
        self.spawned_order.try_reserve(1).map_err(|error| format!("spawned entity list: {error}"))?;
        self.advance(id, Stage::Initialized, Stage::Spawned)?;
        let index = self.spawned_order.len();
        self.spawned_order.push(id);
        self.slots[id.0 as u32 as usize].entry.as_mut().expect("spawned entity exists").spawned_index = Some(index);
        Ok(())
    }

    /// Actual spawn order, subsequently changed by native swap-removal.
    pub fn spawned_entities(&self) -> &[EntityId] { &self.spawned_order }

    /// 1408ac580 scans the live spawned list in its actual order and compares
    /// interned tag identities exactly; callers dispatch the real remove
    /// event and must reject SPAWN_TAG_MAIN as that function does.
    pub fn next_spawned_with_tag(&self, from: usize, tag: NameId) -> Result<Option<(usize, EntityId)>, String> {
        for (index, id) in self.spawned_order.iter().copied().enumerate().skip(from) {
            if self.spawn_tag(id)? == tag { return Ok(Some((index, id))); }
        }
        Ok(None)
    }

    /// 1407b5f50 visits occupied native managed slots in ascending order.
    /// The cursor is a slot index, not spawn order or a map-name ordering.
    pub fn next_managed_entity(&self, from: usize) -> Option<(usize, EntityId)> {
        self.slots.iter().enumerate().skip(from).find_map(|(index, slot)| {
            slot.entry.as_ref()?;
            Some((index, EntityId((u64::from(slot.generation) << 32) | index as u64)))
        })
    }

    pub fn find(&self, name: &str) -> Option<EntityId> { self.by_name.get(&NameQuery(name)).copied() }

    pub fn ai_world_index(&self, id: EntityId) -> Result<u32, String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("AIWorld entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("AIWorld entity generation stale".into()); }
        Ok(slot.entry.as_ref().ok_or("AIWorld entity retired")?.ai_world_index)
    }
    pub fn set_ai_world_index(&mut self, id: EntityId, index: u32) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("AIWorld entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("AIWorld entity generation stale".into()); }
        slot.entry.as_mut().ok_or("AIWorld entity retired")?.ai_world_index = index;
        Ok(())
    }
    ///875fc0: queue the actual EC(old,new) packet before publishing2e0.
    ///Birth assignments use set_ai_world_index before component admission.
    pub fn change_ai_world_index(&mut self,id:EntityId,index:u32,
        events:&mut super::entity_events::EntityEvents)->Result<bool,String>{
        let previous=self.ai_world_index(id)?;
        if previous==index{return Ok(false)}
        let mut bytes=Vec::with_capacity(8);
        bytes.extend_from_slice(&previous.to_le_bytes());bytes.extend_from_slice(&index.to_le_bytes());
        events.post(super::entity_events::Event{id:0xec,target:Some(id),payload:super::entity_events::Payload::Bytes(bytes)});
        self.set_ai_world_index(id,index)?;
        Ok(true)
    }

    pub fn stage(&self, id: EntityId) -> Option<Stage> {
        let slot = self.slots.get(id.0 as u32 as usize)?;
        (slot.generation == (id.0 >> 32) as u32).then(|| slot.entry.as_ref().map(|entry| entry.stage)).flatten()
    }

    /// Resolve the original +ac managed reference through its actual generation.
    /// A retired owner resolves null; it never transfers to a reused slot.
    pub fn owner(&self, id: EntityId) -> Result<Option<EntityId>, String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity owner slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("entity owner generation stale".into()); }
        let entry = slot.entry.as_ref().ok_or("entity owner subject retired")?;
        Ok(entry.owner.filter(|owner| self.get(*owner).is_some()))
    }
    pub fn set_owner(&mut self, id: EntityId, owner: Option<EntityId>) -> Result<(), String> {
        if owner.is_some_and(|owner| self.get(owner).is_none()) { return Err("assigned entity owner is stale".into()); }
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity owner slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("entity owner generation stale".into()); }
        slot.entry.as_mut().ok_or("entity owner subject retired")?.owner = owner;
        Ok(())
    }

    pub fn get(&self, id: EntityId) -> Option<(&str, usize)> {
        let slot = self.slots.get(id.0 as u32 as usize)?;
        if slot.generation != (id.0 >> 32) as u32 { return None; }
        let entry = slot.entry.as_ref()?;
        Some((&entry.name, entry.class))
    }

    pub fn spawn_tag(&self, id: EntityId) -> Result<NameId, String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        Ok(slot.entry.as_ref().ok_or("entity already retired")?.spawn_tag)
    }

    pub(crate) fn entity_context(&self, id: EntityId) -> Result<EntityContext, String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        Ok(slot.entry.as_ref().ok_or("entity already retired")?.context)
    }

    /// 87c730 snapshots the actual virtual+40 result during inherited Spawn,
    /// before components and derived callbacks can request another context.
    pub(crate) fn initialize_entity_context(&mut self, id: EntityId, current: u32) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_mut().ok_or("entity already retired")?;
        if entry.stage != Stage::Initialized { return Err("entity context initialization is outside inherited Spawn".into()); }
        entry.context = EntityContext { current, requested: current, previous: current };
        Ok(())
    }

    /// 87ce50 writes the requested context and 8a62e0 enqueues the actual
    /// managed identity once, even if callbacks request it repeatedly.
    pub(crate) fn set_requested_entity_context(&mut self, id: EntityId, requested: u32) -> Result<(), String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        if slot.entry.is_none() { return Err("entity already retired".into()); }
        let append = !self.pending_contexts.contains(&Some(id));
        if append { self.pending_contexts.try_reserve(1).map_err(|error| format!("entity context queue: {error}"))?; }
        let slot = &mut self.slots[id.0 as u32 as usize];
        slot.entry.as_mut().ok_or("entity already retired")?.context.requested = requested;
        if append { self.pending_contexts.push(Some(id)); }
        Ok(())
    }

    /// A source callback on an actual unregistered entity queues the native
    /// invalid managed ID; its destructor removes that same sentinel.
    pub(crate) fn set_unregistered_entity_context(&mut self, state: &mut UnregisteredEntityState, requested: u32) -> Result<(), String> {
        if !self.pending_contexts.contains(&None) {
            self.pending_contexts.try_reserve(1).map_err(|error| format!("entity context queue: {error}"))?;
            self.pending_contexts.push(None);
        }
        state.context.requested = requested;
        Ok(())
    }
    pub(crate) fn remove_unregistered_entity_context(&mut self) { self.remove_context_target(None); }
    /// 875f50/8a6260 share the game-local entity-update queue with the
    /// destructor and the 8ab380 post-load update-list pass.
    pub(crate) fn add_entity_update_flags(&mut self, id: EntityId, flags: u32) -> Result<(), String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_ref().ok_or("entity already retired")?;
        let old = entry.update_mask;
        let next = old | flags;
        let active = entry.update_active;
        let append = next != 0 && !self.pending_entity_updates.contains(&Some(id));
        if append { self.pending_entity_updates.try_reserve(1).map_err(|error| format!("entity update queue: {error}"))?; }
        self.slots[id.0 as u32 as usize].entry.as_mut().expect("validated entity").update_mask = next;
        if next != 0 {
            if old == 0 && active { self.pending_update_removals = self.pending_update_removals.wrapping_sub(1); }
            if append { self.pending_entity_updates.push(Some(id)); }
        }
        Ok(())
    }
    /// 875f90 postpones active-list removal until the post-load pass.
    pub(crate) fn clear_entity_update_flags(&mut self, id: EntityId, flags: u32) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_mut().ok_or("entity already retired")?;
        if entry.update_mask != 0 {
            entry.update_mask &= !flags;
            if entry.update_mask == 0 && entry.update_active {
                self.pending_update_removals = self.pending_update_removals.wrapping_add(1);
            }
        }
        Ok(())
    }
    pub(crate) fn add_unregistered_entity_update_flags(&mut self, state: &mut UnregisteredEntityState, flags: u32) -> Result<(), String> {
        let next = state.update_mask | flags;
        if next != 0 && !self.pending_entity_updates.contains(&None) {
            self.pending_entity_updates.try_reserve(1).map_err(|error| format!("entity update queue: {error}"))?;
            self.pending_entity_updates.push(None);
        }
        state.update_mask = next;
        Ok(())
    }
    pub(crate) fn clear_unregistered_entity_update_flags(&mut self, state: &mut UnregisteredEntityState, flags: u32) {
        state.update_mask &= !flags;
    }
    pub(crate) fn remove_unregistered_entity_update(&mut self) {
        if let Some(index) = self.pending_entity_updates.iter().position(Option::is_none) { self.pending_entity_updates.remove(index); }
    }
    pub(crate) fn adopt_unregistered_entity_state(&mut self, id: EntityId, state: &UnregisteredEntityState) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_mut().ok_or("entity already retired")?;
        if entry.stage != Stage::Allocated || entry.context != EntityContext::default() || entry.update_mask != 0 || entry.update_active {
            return Err("entity registration state differs from constructor".into());
        }
        entry.context = state.context;
        entry.update_mask = state.update_mask;
        Ok(())
    }
    /// 8ab380 removes inactive active-list entries in reverse, then admits
    /// queued valid identities in source order; stale/native-invalid IDs skip.
    pub(crate) fn flush_entity_updates(&mut self) -> Result<(), String> {
        if self.pending_update_removals != 0 {
            for position in (0..self.active_entity_updates.len()).rev() {
                let Some(id) = self.active_entity_updates[position] else { continue };
                let Some(slot) = self.slots.get_mut(id.0 as u32 as usize) else { continue };
                if slot.generation != (id.0 >> 32) as u32 { continue; }
                let Some(entry) = slot.entry.as_mut() else { continue };
                if entry.update_mask == 0 {
                    entry.update_active = false;
                    self.active_entity_updates[position] = None;
                }
            }
            self.pending_update_removals = 0;
        }
        self.active_entity_updates.try_reserve(self.pending_entity_updates.len())
            .map_err(|error| format!("entity active update list: {error}"))?;
        for target in &self.pending_entity_updates {
            let Some(id) = *target else { continue };
            let Some(slot) = self.slots.get_mut(id.0 as u32 as usize) else { continue };
            if slot.generation != (id.0 >> 32) as u32 { continue; }
            let Some(entry) = slot.entry.as_mut() else { continue };
            if !entry.update_active {
                entry.update_active = true;
                self.active_entity_updates.push(Some(id));
            }
        }
        self.pending_entity_updates.clear();
        Ok(())
    }
    fn remove_context_target(&mut self, target: Option<EntityId>) {
        if let Some(index) = self.pending_contexts.iter().position(|value| *value == target) {
            self.pending_contexts.remove(index);
            if index < self.context_cursor { self.context_cursor -= 1; }
        }
    }
    pub(crate) fn next_context_request(&self) -> Option<Option<EntityId>> {
        self.pending_contexts.get(self.context_cursor).copied()
    }
    pub(crate) fn has_context_requests(&self) -> bool { !self.pending_contexts.is_empty() }
    pub(crate) fn has_context_touched(&self) -> bool { !self.context_touched.is_empty() }
    pub(crate) fn has_pending_removals(&self) -> bool {
        !self.pending_removals.is_empty() || self.active_removal_batch.is_some() || self.removal_batch_in_flight
    }
    pub(crate) fn acknowledge_context_request(&mut self, id: EntityId) -> Result<(), String> {
        if self.next_context_request() != Some(Some(id)) { return Err("entity context cursor differs".into()); }
        if !self.context_touched.contains(&id) {
            self.context_touched.try_reserve(1).map_err(|error| format!("entity context touched list: {error}"))?;
            self.context_touched.push(id);
        }
        self.context_cursor += 1;
        Ok(())
    }
    pub(crate) fn finish_context_round(&mut self) -> Result<(), String> {
        if self.context_cursor != self.pending_contexts.len() { return Err("entity context round is incomplete".into()); }
        self.pending_contexts.clear();
        self.context_cursor = 0;
        Ok(())
    }
    pub(crate) fn next_context_commit(&self) -> Option<EntityId> { self.context_touched.get(self.context_commit_cursor).copied() }
    pub(crate) fn acknowledge_context_commit(&mut self, id: EntityId) -> Result<(), String> {
        if self.next_context_commit() != Some(id) { return Err("entity context commit cursor differs".into()); }
        self.commit_entity_context(id)?;
        self.context_commit_cursor += 1;
        Ok(())
    }
    pub(crate) fn finish_context_commits(&mut self) -> Result<(), String> {
        if self.context_commit_cursor != self.context_touched.len() { return Err("entity context commits are incomplete".into()); }
        Ok(())
    }
    pub(crate) fn next_context_notification(&self) -> Option<EntityId> { self.context_touched.get(self.context_notify_cursor).copied() }
    pub(crate) fn acknowledge_context_notification(&mut self, id: EntityId) -> Result<(), String> {
        if self.next_context_notification() != Some(id) { return Err("entity context notification cursor differs".into()); }
        self.context_notify_cursor += 1;
        Ok(())
    }
    pub(crate) fn finish_context_notifications(&mut self) -> Result<(), String> {
        if self.context_notify_cursor != self.context_touched.len() { return Err("entity context notifications are incomplete".into()); }
        self.context_touched.clear();
        self.context_commit_cursor = 0;
        self.context_notify_cursor = 0;
        Ok(())
    }

    /// 878fb0 is called only for the context batch's actual queued members.
    pub(crate) fn commit_entity_context(&mut self, id: EntityId) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let context = &mut slot.entry.as_mut().ok_or("entity already retired")?.context;
        context.previous = context.current;
        context.current = context.requested;
        Ok(())
    }

    ///Borrow the already-published spelling from the canonical tag owner.
    pub fn spawn_tag_text(&self,id:EntityId)->Result<&str,String>{
        let slot=self.slots.get(id.0 as u32 as usize).ok_or("entity tag slot absent")?;
        if slot.generation!=(id.0>>32)as u32{return Err("entity tag generation stale".into());}
        let entry=slot.entry.as_ref().ok_or("entity tag owner retired")?;
        let group=entry.tag_group.ok_or("entity tag is not published")?;
        Ok(&self.tag_groups.get(group as usize).ok_or("entity tag group absent")?.spelling)
    }
    /// Reflected storage before the spawn callback publishes the tag index.
    /// Later changes must go through the indexed tag owner (14087c360).
    pub fn set_unpublished_spawn_tag(&mut self, id: EntityId, tag: NameId) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_mut().ok_or("entity already retired")?;
        if entry.stage == Stage::Spawned { return Err("spawned tag requires indexed update".into()); }
        entry.spawn_tag = tag;
        Ok(())
    }

    /// Publish the current interned tag after spawn-list insertion, matching
    /// 14087c360/1408ac180. `text` must be resolved from that same name pool.
    pub fn publish_spawn_tag(&mut self, id: EntityId, text: &str) -> Result<(), String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_ref().ok_or("entity already retired")?;
        if entry.stage != Stage::Spawned { return Err("tag publication requires spawned entity".into()); }
        if entry.tag_group.is_some() { return Ok(()); }
        let group_id = self.prepare_tag_group(text)?;
        self.tag_groups[group_id as usize].members.push(id);
        self.slots[id.0 as u32 as usize].entry.as_mut().expect("validated entity").tag_group = Some(group_id);
        Ok(())
    }

    /// 14087c360 removes the old published entry, inserts the new one, then
    /// updates the token. Reserve first so allocation failure leaves the live
    /// entity and old index entry untouched. Text comes from the shared pool.
    pub fn set_published_spawn_tag(&mut self, id: EntityId, tag: NameId, text: &str) -> Result<(), String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_ref().ok_or("entity already retired")?;
        let old_group = entry.tag_group.ok_or("entity tag is not published")?;
        let old_position = self.tag_groups.get(old_group as usize).ok_or("old tag group absent")?.members
            .iter().position(|member| *member == id).ok_or("entity missing from old tag group")?;
        let new_group = self.prepare_tag_group(text)?;
        self.tag_groups[old_group as usize].members.swap_remove(old_position);
        self.tag_groups[new_group as usize].members.push(id);
        let entry = self.slots[id.0 as u32 as usize].entry.as_mut().expect("validated entity");
        entry.tag_group = Some(new_group);
        entry.spawn_tag = tag;
        Ok(())
    }

    /// 1408a8900: hash the exact bytes, then compare only entries in that
    /// bucket case-insensitively. The caller borrows the actual member order.
    pub fn tagged_entities(&self, text: &str) -> Option<&[EntityId]> {
        let hash = text.bytes().take_while(|&byte| byte != 0)
            .fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(u32::from(byte)));
        let mut current = self.tag_heads[(hash & 0xff) as usize];
        while current != u32::MAX {
            let group = &self.tag_groups[current as usize];
            match tag_compare(group.spelling.as_bytes(), text.as_bytes()) {
                std::cmp::Ordering::Equal => return Some(&group.members),
                std::cmp::Ordering::Greater => return None,
                std::cmp::Ordering::Less => current = group.next,
            }
        }
        None
    }

    fn prepare_tag_group(&mut self, text: &str) -> Result<u32, String> {
        let hash = text.bytes().take_while(|&byte| byte != 0)
            .fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(u32::from(byte)));
        let bucket = (hash & 0xff) as usize;
        let mut current = self.tag_heads[bucket];
        let mut previous = u32::MAX;
        while current != u32::MAX {
            let group = &self.tag_groups[current as usize];
            let order = tag_compare(group.spelling.as_bytes(), text.as_bytes());
            if order == std::cmp::Ordering::Equal { break; }
            if order == std::cmp::Ordering::Greater { break; }
            previous = current;
            current = group.next;
        }
        let group_id = if current != u32::MAX && tag_compare(self.tag_groups[current as usize].spelling.as_bytes(), text.as_bytes()) == std::cmp::Ordering::Equal {
            current
        } else {
            let group_id = u32::try_from(self.tag_groups.len()).map_err(|_| "spawn tag group limit exceeded")?;
            if group_id == u32::MAX { return Err("spawn tag group limit exceeded".into()); }
            self.tag_groups.try_reserve(1).map_err(|error| format!("spawn tag groups: {error}"))?;
            let mut members = Vec::new();
            members.try_reserve(1).map_err(|error| format!("spawn tag members: {error}"))?;
            self.tag_groups.push(TagGroup { spelling: Arc::from(text), members, next: current });
            if previous == u32::MAX { self.tag_heads[bucket] = group_id; }
            else { self.tag_groups[previous as usize].next = group_id; }
            group_id
        };
        let members = &mut self.tag_groups[group_id as usize].members;
        members.try_reserve(1).map_err(|error| format!("spawn tag members: {error}"))?;
        Ok(group_id)
    }

    /// 14065afa0 records the actual entity's retention bit before its
    /// components are released. Retrying this operation is idempotent.
    pub fn prepare_entity_release(&mut self, id: EntityId, runtime_flags: u8) -> Result<(), String> {
        let index = id.0 as u32 as usize;
        let slot = self.slots.get(index).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_ref().ok_or("entity already retired")?;
        if !entry.removal_requested || entry.removal_queued { return Err("entity release was not snapshotted".into()); }
        let retained = runtime_flags & 2 != 0;
        let name = entry.name.clone();
        if retained && !self.removed_names.iter().any(|existing| existing.as_ref() == name.as_ref()) {
            self.removed_names.try_reserve(1).map_err(|error| format!("retained entity names: {error}"))?;
            self.removed_names.push(name);
        }
        self.slots[index].reserved = retained;
        self.slots[index].entry.as_mut().expect("validated live entity").release_prepared = true;
        Ok(())
    }

    /// GameLocal save-manager names, in insertion order, retained until the
    /// owner is destroyed. Restoration must dispatch the real remove event.
    pub fn removed_entity_names(&self) -> &[Arc<str>] { &self.removed_names }

    /// Ordered live children owned by the entity, not authored parent names.
    /// A stale identity is an error rather than an assumed empty hierarchy.
    pub fn children(&self, id: EntityId) -> Result<&[EntityId], String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        Ok(&slot.entry.as_ref().ok_or("entity already retired")?.children)
    }

    /// 878fd0 appends a real child to its parent's ordered list. The
    /// reference-frame owner performs this only after its other attachment
    /// preconditions have been validated.
    pub(crate) fn link_child(&mut self, parent: EntityId, child: EntityId) -> Result<(), String> {
        if parent == child { return Err("entity cannot parent itself".into()); }
        let child_index = child.0 as u32 as usize;
        if self.get(child).is_none() { return Err("child identity is stale".into()); }
        if self.slots[child_index].entry.as_ref().ok_or("child retired")?.parent.is_some() {
            return Err("child already has a parent".into());
        }
        let mut ancestor = Some(parent);
        let mut remaining = self.slots.len();
        while let Some(id) = ancestor {
            if id == child { return Err("entity attachment would form a cycle".into()); }
            if remaining == 0 { return Err("entity attachment hierarchy is cyclic".into()); }
            remaining -= 1;
            if self.get(id).is_none() { return Err("attachment ancestor identity is stale".into()); }
            ancestor = self.slots[id.0 as u32 as usize].entry.as_ref().ok_or("attachment ancestor retired")?.parent;
        }
        let parent_index = parent.0 as u32 as usize;
        if self.get(parent).is_none() || self.get(child).is_none() { return Err("attachment identity is stale".into()); }
        let children = &mut self.slots[parent_index].entry.as_mut().ok_or("parent retired")?.children;
        if children.contains(&child) { return Err("child is already in the parent list".into()); }
        children.try_reserve(1).map_err(|e| format!("entity child list: {e}"))?;
        children.push(child);
        self.slots[child_index].entry.as_mut().ok_or("child retired")?.parent = Some(parent);
        Ok(())
    }

    /// 879100 removes the matching child while retaining sibling order.
    /// The reference-frame owner supplies the real visibility, time and event
    /// effects around this list operation.
    pub(crate) fn unlink_child(&mut self, parent: EntityId, child: EntityId) -> Result<(), String> {
        if self.parent(child)? != Some(parent) { return Err("child parent identity mismatch".into()); }
        let parent_index = parent.0 as u32 as usize;
        let child_index = child.0 as u32 as usize;
        if self.get(parent).is_none() { return Err("parent identity is stale".into()); }
        let children = &mut self.slots[parent_index].entry.as_mut().ok_or("parent retired")?.children;
        let index = children.iter().position(|&id| id == child).ok_or("child is absent from parent list")?;
        children.remove(index);
        self.slots[child_index].entry.as_mut().ok_or("child retired")?.parent = None;
        Ok(())
    }

    /// The live immediate attachment, not an authored parent name.
    pub fn parent(&self, id: EntityId) -> Result<Option<EntityId>, String> {
        let slot = self.slots.get(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let parent = slot.entry.as_ref().ok_or("entity already retired")?.parent;
        // 6dc560 resolves the checked handle through the managed table; a
        // retired parent resolves to null, not another generation in its slot.
        Ok(parent.filter(|parent| self.get(*parent).is_some()))
    }

    /// The base idEntity remove event (1408794b0) queues the managed handle
    /// only on the first request; the bit remains set until destruction.
    pub fn request_removal(&mut self, id: EntityId) -> Result<(), String> {
        let slot = self.slots.get_mut(id.0 as u32 as usize).ok_or("entity slot absent")?;
        if slot.generation != (id.0 >> 32) as u32 { return Err("stale entity identity".into()); }
        let entry = slot.entry.as_mut().ok_or("entity already retired")?;
        if entry.removal_requested { return Ok(()); }
        self.pending_removals.try_reserve(1)
            .map_err(|error| format!("entity removal queue allocation: {error}"))?;
        self.pending_removals.push(id);
        entry.removal_requested = true;
        entry.removal_queued = true;
        Ok(())
    }

    /// 1408ab380 swaps out the ordered pending list before invoking release
    /// callbacks. Requests made during that batch remain for the next one.
    /// Allocation must succeed before removing any IDs from the real queue.
    pub fn take_removal_batch(&mut self) -> Result<Option<super::entity_release::EntityReleaseBatch>, String> {
        if self.pending_removals.is_empty() { return Ok(None); }
        let batch = super::entity_release::EntityReleaseBatch::from_snapshot(&self.pending_removals)?;
        for id in &self.pending_removals {
            let slot = self.slots.get(id.0 as u32 as usize).ok_or("queued entity slot absent")?;
            if slot.generation != (id.0 >> 32) as u32 { return Err("stale queued entity identity".into()); }
            let entry = slot.entry.as_ref().ok_or("queued entity already retired")?;
            if !entry.removal_requested || !entry.removal_queued { return Err("inconsistent entity removal queue".into()); }
        }
        for id in &self.pending_removals {
            let entry = self.slots[id.0 as u32 as usize].entry.as_mut().expect("validated queued entity");
            entry.removal_queued = false;
        }
        self.pending_removals.clear();
        Ok(Some(batch))
    }

    /// Retain the active 8ab380 removal snapshot across asynchronous release
    /// callbacks while permitting new requests to enter the real queue.
    pub(crate) fn take_active_removal_batch(&mut self) -> Result<Option<super::entity_release::EntityReleaseBatch>, String> {
        if self.removal_batch_in_flight { return Err("entity removal batch is already running".into()); }
        if self.active_removal_batch.is_none() { self.active_removal_batch = self.take_removal_batch()?; }
        let batch = self.active_removal_batch.take();
        self.removal_batch_in_flight = batch.is_some();
        Ok(batch)
    }
    pub(crate) fn active_removal_pending(&self) -> bool {
        self.active_removal_batch.is_some() || self.removal_batch_in_flight
    }
    pub(crate) fn finish_active_removal_batch(&mut self, batch: super::entity_release::EntityReleaseBatch,
        completed: bool) -> Result<(), String> {
        if !self.removal_batch_in_flight || self.active_removal_batch.is_some() {
            return Err("entity removal batch ownership differs".into());
        }
        self.removal_batch_in_flight = false;
        if !completed { self.active_removal_batch = Some(batch); }
        Ok(())
    }

    /// Called after component and graph owners have detached their callbacks.
    pub fn retire(&mut self, id: EntityId) -> Option<Arc<str>> {
        self.retire_inner(id, Retirement::Ordinary)
    }

    /// Called only by the completed release batch after its shared drain and
    /// actual component/graph/attachment validation have succeeded.
    pub fn retire_after_release(&mut self, id: EntityId) -> Option<Arc<str>> {
        self.retire_inner(id, Retirement::QueuedRelease)
    }

    /// Called by whole-world destruction only after its distinct component
    /// drain and manager phases. It does not perform save-game preparation.
    pub fn retire_for_world_shutdown(&mut self, id: EntityId) -> Option<Arc<str>> {
        self.retire_inner(id, Retirement::WorldShutdown)
    }

    fn retire_inner(&mut self, id: EntityId, retirement: Retirement) -> Option<Arc<str>> {
        let index = id.0 as u32;
        let slot = self.slots.get(index as usize)?;
        if slot.generation != (id.0 >> 32) as u32 { return None; }
        let entry = slot.entry.as_ref()?;
        if entry.parent.is_some() || !entry.children.is_empty() { return None; }
        match retirement {
            Retirement::Ordinary if entry.removal_requested || entry.removal_queued => return None,
            Retirement::QueuedRelease if !entry.removal_requested || entry.removal_queued || !entry.release_prepared => return None,
            _ => {}
        }
        let tag_position = match entry.tag_group {
            Some(group_id) => Some((group_id, self.tag_groups.get(group_id as usize)?.members.iter().position(|member| *member == id)?)),
            None => None,
        };
        let update_mask = entry.update_mask;
        let update_active = entry.update_active;
        if let Some(position) = entry.spawned_index {
            if self.spawned_order.get(position) != Some(&id) { return None; }
            let last = *self.spawned_order.last()?;
            if last != id {
                let last_slot = self.slots.get(last.0 as u32 as usize)?;
                if last_slot.generation != (last.0 >> 32) as u32
                    || last_slot.entry.as_ref()?.spawned_index != Some(self.spawned_order.len() - 1) { return None; }
                self.slots[last.0 as u32 as usize].entry.as_mut()?.spawned_index = Some(position);
            }
            self.spawned_order.swap_remove(position);
        }
        if let Some((group_id, position)) = tag_position {
            self.tag_groups[group_id as usize].members.swap_remove(position);
        }
        if update_mask != 0 && update_active {
            self.pending_update_removals = self.pending_update_removals.wrapping_add(1);
        }
        if update_active {
            if let Some(position) = self.active_entity_updates.iter().position(|target| *target == Some(id)) {
                self.active_entity_updates[position] = None;
            }
        }
        if let Some(position) = self.pending_entity_updates.iter().position(|target| *target == Some(id)) {
            self.pending_entity_updates.remove(position);
        }
        self.remove_context_target(Some(id));
        if let Some(position) = self.context_touched.iter().position(|value| *value == id) {
            self.context_touched.remove(position);
            if position < self.context_commit_cursor { self.context_commit_cursor -= 1; }
            if position < self.context_notify_cursor { self.context_notify_cursor -= 1; }
        }
        self.detach_event_observers(id)?;
        let entry = self.slots[index as usize].entry.take()?;
        self.by_name.remove(&NameQuery(&entry.name));
        Some(entry.name)
    }
}

#[derive(Clone, Copy)]
enum Retirement { Ordinary, QueuedRelease, WorldShutdown }
