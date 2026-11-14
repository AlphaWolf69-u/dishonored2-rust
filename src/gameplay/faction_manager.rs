//! Source-backed arkFactionManager resource owner (6038f0/604f90/604c60).
//! The caller supplies the actual registered cpntFaction declarations in the
//! declaration-class registry's order; this owner never scans the catalog.
use std::{collections::{HashMap, HashSet}, sync::Arc};
use crate::assets::entities::spawn::ComponentDeclarationLease;
use crate::gameplay::{entity_lifetime::EntityId, faction_registry::ResourceName};

pub(crate) const ALLY: u32 = 0;
pub(crate) const UNKNOWN: u32 = 3;

/// Compact fields projected from the existing resolved declaration and its
/// retained inheritance lease and the session declaration-name namespace.
pub(crate) struct Definition {
    pub lease: Arc<ComponentDeclarationLease>,
    pub name: ResourceName,
    pub parent: Option<ResourceName>,
    /// Text of the declaration's m_factionName; this is not a component or
    /// managed-entity identity.
    pub faction_name: Arc<str>,
    pub default_relationship: u32,
    /// 604dc0 tests these authored targets in declaration order.
    pub relationships: Vec<(ResourceName, u32)>,
    pub propagate_overrides: bool,
}
struct Row {
    declaration: Arc<Definition>,
    baseline: Vec<u32>,
    current: Vec<u32>,
    members: Vec<EntityId>,
}
struct EntityRecord {
    faction: usize,
    /// 605990/6060a0 preserve insertion order and shift on removal.
    overrides: Vec<(EntityId, u32)>,
}

/// Actual 606d10 sends an entity event and a reflected graph notification.
/// The live dispatcher supplies this host; it may not be replaced by a no-op.
pub(crate) trait Notifications {
    fn relationship_changed(&mut self, source: EntityId, target: EntityId, value: u32)
        -> Result<(), String>;
}

#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub(crate) struct FactionManager {
    rows: Vec<Row>,
    indices: HashMap<ResourceName, usize>,
    entities: HashMap<EntityId, EntityRecord>,
}
impl FactionManager {
    /// Observation excludes host ordinals: resource names and named-pair
    /// relationships remain comparable across native swap/purge/reload order.
    pub(crate) fn semantic_signature(&self) -> blake3::Hash {
        let mut rows = self.rows.iter().collect::<Vec<_>>();
        rows.sort_unstable_by(|a, b| a.declaration.name.as_str().cmp(b.declaration.name.as_str()));
        let mut hash = blake3::Hasher::new();
        for source in &rows {
            hash.update(source.declaration.name.as_str().as_bytes()); hash.update(&[0]);
            hash.update(source.declaration.faction_name.as_bytes()); hash.update(&[0]);
            hash.update(&source.declaration.default_relationship.to_le_bytes());
            hash.update(&[u8::from(source.declaration.propagate_overrides)]);
            for target in &rows {
                hash.update(target.declaration.name.as_str().as_bytes()); hash.update(&[0]);
                hash.update(&source.current[self.indices[&target.declaration.name]].to_le_bytes());
            }
        }
        hash.finalize()
    }

    pub(crate) fn has_members(&self) -> bool { !self.entities.is_empty() }
    pub(crate) fn verify_index(&self, name: &ResourceName, index: u8) -> Result<(), String> {
        let (canonical, &actual) = self.indices.get_key_value(name).ok_or("faction is absent from manager")?;
        if !canonical.same_identity(name) || actual != index as usize {
            return Err("faction class index changed after component initialization".into());
        }
        Ok(())
    }
    pub(crate) fn preflight_admit(&self, entity: EntityId, name: &ResourceName, index: u8) -> Result<(), String> {
        self.verify_index(name, index)?;
        if self.entities.contains_key(&entity) { return Err("entity faction already admitted".into()); }
        Ok(())
    }
    /// 604f90 enumerates the already-registered declaration class, initializes
    /// a row per declaration, and resolves explicit/inherited relationships.
    pub(super) fn initialize(registered: Vec<Arc<Definition>>) -> Result<Self, String> {
        if registered.len() > i8::MAX as usize + 1 { return Err("faction count exceeds native signed index".into()); }
        let mut indices = HashMap::with_capacity(registered.len());
        for (index, definition) in registered.iter().enumerate() {
            if definition.name.as_str().is_empty() { return Err("unnamed registered faction is not hosted".into()); }
            if indices.insert(definition.name.clone(), index).is_some() { return Err("duplicate registered faction identity".into()); }
            if definition.default_relationship > UNKNOWN || definition.relationships.iter().any(|(_, value)| *value > UNKNOWN) {
                return Err("invalid faction relationship value".into());
            }
        }
        for definition in &registered {
            let valid = |name: &ResourceName| indices.get_key_value(name)
                .is_some_and(|(canonical, _)| canonical.same_identity(name));
            if definition.parent.as_ref().is_some_and(|parent| !valid(parent)) {
                return Err("registered faction parent is unavailable or belongs to another session".into());
            }
            if definition.relationships.iter().any(|(target, _)| !valid(target)) {
                return Err("registered faction relationship target is unavailable or belongs to another session".into());
            }
            let mut visited = HashSet::new();
            let mut current = Some(definition.name.clone());
            while let Some(name) = current {
                if !visited.insert(name.clone()) { return Err("faction inheritance cycle".into()); }
                current = registered[*indices.get(&name).ok_or("faction parent missing")?].parent.clone();
            }
        }
        fn inherits(defs: &[Arc<Definition>], map: &HashMap<ResourceName,usize>,
            child: usize, ancestor: usize) -> bool {
            let mut next = defs[child].parent.clone();
            while let Some(name) = next {
                let index = map[&name];
                if index == ancestor { return true; }
                next = defs[index].parent.clone();
            }
            false
        }
        fn inherited_relation(defs: &[Arc<Definition>], map: &HashMap<ResourceName,usize>,
            source: usize, target: usize, stack: &mut HashSet<(usize, usize)>) -> Result<u32, String> {
            if !stack.insert((source, target)) { return Err("faction relationship inheritance cycle".into()); }
            let row = &defs[source];
            let value = if let Some((_, value)) = row.relationships.iter().find(|(name, _)| name == &defs[target].name) {
                *value
            } else {
                let mut found = UNKNOWN;
                if let Some(parent) = defs[target].parent.as_ref() {
                    found = inherited_relation(defs, map, source, map[parent], stack)?;
                }
                if found == UNKNOWN {
                    if let Some(parent) = row.parent.as_ref() {
                        found = inherited_relation(defs, map, map[parent], target, stack)?;
                    }
                }
                found
            };
            stack.remove(&(source, target));
            Ok(value)
        }
        let mut rows = Vec::with_capacity(registered.len());
        for source in 0..registered.len() {
            let mut baseline = Vec::with_capacity(registered.len());
            for target in 0..registered.len() {
                let value = if source == target || inherits(&registered, &indices, source, target)
                    || inherits(&registered, &indices, target, source) { ALLY }
                else {
                    let value = inherited_relation(&registered, &indices, source, target, &mut HashSet::new())?;
                    if value == UNKNOWN { registered[source].default_relationship } else { value }
                };
                baseline.push(value);
            }
            rows.push(Row { declaration: registered[source].clone(), current: baseline.clone(), baseline, members: Vec::new() });
        }
        Ok(Self { rows, indices, entities: HashMap::new() })
    }

    /// 605c20 registers the real spawned faction component's entity. Its
    /// declaration-controlled propagation copies the first same-faction
    /// member's outgoing overrides and incoming overrides to the new member.
    pub(crate) fn admit(&mut self, entity: EntityId, faction: &ResourceName) -> Result<(), String> {
        if self.entities.contains_key(&entity) { return Err("entity faction already admitted".into()); }
        let (canonical, &index) = self.indices.get_key_value(faction).ok_or("entity faction declaration not registered")?;
        if !canonical.same_identity(faction) { return Err("entity faction belongs to another session".into()); }
        self.entities.try_reserve(1).map_err(|e| format!("faction entity allocation: {e}"))?;
        self.rows[index].members.try_reserve(1).map_err(|e| format!("faction member allocation: {e}"))?;
        let representative = self.rows[index].members.first().copied()
            .filter(|_| self.rows[index].declaration.propagate_overrides);
        let mut outgoing = Vec::new();
        if let Some(first) = representative {
            outgoing = self.entities.get(&first).ok_or("faction representative is absent")?.overrides.clone();
            for record in self.entities.values_mut() {
                if record.overrides.iter().any(|(target, _)| *target == first) {
                    record.overrides.try_reserve(1).map_err(|e| format!("faction override allocation: {e}"))?;
                }
            }
        }
        if let Some(first) = representative {
            for record in self.entities.values_mut() {
                if let Some((_, value)) = record.overrides.iter().find(|(target, _)| *target == first).copied() {
                    record.overrides.push((entity, value));
                }
            }
        }
        self.entities.insert(entity, EntityRecord { faction: index, overrides: outgoing });
        self.rows[index].members.push(entity);
        Ok(())
    }

    /// 604c60 first checks source's entity override, then the directional
    /// faction matrix. A missing source/target has the native value 3.
    pub(crate) fn relationship(&self, source: EntityId, target: EntityId) -> u32 {
        let Some(source) = self.entities.get(&source) else { return UNKNOWN };
        if let Some((_, value)) = source.overrides.iter().find(|(entity, _)| *entity == target) {
            if *value != UNKNOWN { return *value; }
        }
        let Some(target) = self.entities.get(&target) else { return UNKNOWN };
        self.rows[source.faction].current[target.faction]
    }

    /// 605770 propagates to other members of each participating faction
    /// before applying the requested pair. The inner 605990 operation owns
    /// each ordered notification; the same entity is an original no-op.
    pub(crate) fn set_entity_relationship(&mut self, source: EntityId, target: EntityId, value: u32,
        notifications: &mut impl Notifications) -> Result<(), String> {
        if source == target { return Ok(()); }
        let Some(source_row) = self.entities.get(&source).map(|entry| entry.faction) else { return Ok(()) };
        let Some(target_row) = self.entities.get(&target).map(|entry| entry.faction) else { return Ok(()) };
        if self.rows[source_row].declaration.propagate_overrides {
            for index in 0..self.rows[source_row].members.len() {
                let member = self.rows[source_row].members[index];
                if member != source { self.set_entity_override(member, target, value, notifications)?; }
            }
        }
        if self.rows[target_row].declaration.propagate_overrides {
            for index in 0..self.rows[target_row].members.len() {
                let member = self.rows[target_row].members[index];
                if member != target { self.set_entity_override(source, member, value, notifications)?; }
            }
        }
        self.set_entity_override(source, target, value, notifications)
    }

    /// 605990 mutates the source entity's ordered override then notifies the
    /// real entity/event-graph owner. A missing source is the original no-op.
    pub(crate) fn set_entity_override(&mut self, source: EntityId, target: EntityId, value: u32,
        notifications: &mut impl Notifications) -> Result<(), String> {
        if value > UNKNOWN { return Err("invalid entity relationship".into()); }
        let Some(record) = self.entities.get_mut(&source) else { return Ok(()) };
        if let Some((_, old)) = record.overrides.iter_mut().find(|(entity, _)| *entity == target) { *old = value; }
        else {
            record.overrides.try_reserve(1).map_err(|e| format!("faction override allocation: {e}"))?;
            record.overrides.push((target, value));
        }
        notifications.relationship_changed(source, target, value)
    }

    /// 6060a0 removes with shifting, then emits the newly resolved relation.
    pub(crate) fn remove_entity_override(&mut self, source: EntityId, target: EntityId,
        notifications: &mut impl Notifications) -> Result<(), String> {
        let Some(record) = self.entities.get_mut(&source) else { return Ok(()) };
        if let Some(index) = record.overrides.iter().position(|(entity, _)| *entity == target) {
            record.overrides.remove(index);
        }
        let value = self.relationship(source, target);
        notifications.relationship_changed(source, target, value)
    }

    /// 605ac0 changes one directional matrix cell then notifies its actual
    /// source/target members in the native nested list order.
    pub(crate) fn set_faction_relationship(&mut self, source: &ResourceName, target: &ResourceName, value: u32,
        notifications: &mut impl Notifications) -> Result<(), String> {
        if value > UNKNOWN { return Err("invalid faction relationship".into()); }
        let (source_id, &a) = self.indices.get_key_value(source).ok_or("source faction is not registered")?;
        let (target_id, &b) = self.indices.get_key_value(target).ok_or("target faction is not registered")?;
        if !source_id.same_identity(source) || !target_id.same_identity(target) {
            return Err("faction identity belongs to another session".into());
        }
        self.rows[a].current[b] = value;
        for &source in &self.rows[a].members {
            for &target in &self.rows[b].members { notifications.relationship_changed(source, target, value)?; }
        }
        Ok(())
    }
    /// 606230 restores the startup matrix value and publishes the same
    /// member-pair notifications as an explicit faction change.
    pub(crate) fn reset_faction_relationship(&mut self, source: &ResourceName, target: &ResourceName,
        notifications: &mut impl Notifications) -> Result<(), String> {
        let value = self.baseline_relationship(source, target)?;
        self.set_faction_relationship(source, target, value, notifications)
    }

    pub(crate) fn baseline_relationship(&self, source: &ResourceName, target: &ResourceName) -> Result<u32, String> {
        let (source_id, &a) = self.indices.get_key_value(source).ok_or("source faction is not registered")?;
        let (target_id, &b) = self.indices.get_key_value(target).ok_or("target faction is not registered")?;
        if !source_id.same_identity(source) || !target_id.same_identity(target) {
            return Err("faction identity belongs to another session".into());
        }
        Ok(self.rows[a].baseline[b])
    }
    /// 6073e0 removes the row member and its own override list; references in
    /// other lists remain generation-bearing until their owners clear them.
    pub(crate) fn retire(&mut self, entity: EntityId) {
        if let Some(record) = self.entities.remove(&entity) {
            if let Some(index) = self.rows[record.faction].members.iter().position(|value| *value == entity) {
                self.rows[record.faction].members.remove(index);
            }
        }
    }
    /// 6071c0/603a40 release rows, overrides and their declaration leases.
    pub(crate) fn clear(&mut self) {
        self.entities.clear();
        self.rows.clear();
        self.indices.clear();
    }
}
