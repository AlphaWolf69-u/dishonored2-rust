//! Usable lifecycle headers over the existing interaction/animation/FX owners.
//! 77b3b0/77b610 retain one mutable usable state, not a second state machine.
use super::*;
use crate::gameplay::{component_state::ComponentInstance,
    entity_lifetime::LiveEntities, name_pool::NameId};

pub(super) struct UsableComponent {
    pub owner: ComponentOwner,
    pub actor: usize,
}
impl ComponentInstance for UsableComponent {
    fn owner(&self) -> &ComponentOwner { &self.owner }
    fn owner_mut(&mut self) -> &mut ComponentOwner { &mut self.owner }
}

impl PhysicsScene {
    pub(super) fn admit_interaction_components(&mut self, document: &entities::Document,
        interactions: &super::super::interactions::Interactions, live: &mut LiveEntities,
        registry: &mut ComponentRegistry, constructor_tag: NameId) -> Result<(), String> {
        let definitions: HashMap<_, _> = document.definitions.iter().filter(|entry| entry.kind == "entity")
            .map(|entry| (entry.id.as_str(), entry)).collect();
        let mut indices: HashMap<_, _> = self.entities.iter().enumerate()
            .map(|(index, entity)| (entity.name.clone(), index)).collect();
        let names: HashMap<_, _> = self.types.classes().hierarchy().registration_order().iter().map(|&index| {
            let metadata = self.types.classes().metadata(index).ok_or("usable class metadata absent")?;
            Ok((self.types.classes().name_text(metadata.name).ok_or("usable class name absent")?.to_vec(), index))
        }).collect::<Result<_, String>>()?;
        for (actor_index, actor) in interactions.actors.iter().enumerate() {
            let index = if let Some(&index) = indices.get(&actor.entity) { index } else {
                let definition = definitions.get(actor.entity.as_str()).ok_or("usable entity declaration absent")?;
                let class = definition.fields.iter().find_map(|(key, value)| match (key.as_str(), value) {
                    ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')),
                    _ => None,
                }).ok_or("usable entity class absent")?;
                let class = *names.get(class.as_bytes()).ok_or("usable entity class is not registered")?;
                let index = self.entities.len();
                let shared = live.allocate_managed(Arc::from(actor.entity.as_str()), class, None, 0, constructor_tag)?;
                self.entities.push(PhysicsEntity { name: actor.entity.clone(), shared,
                    components: EntityComponents::new(index), class, environment: None, flags: 0,
                    entity_flags: 0x001e_8008, entity_flags_d7: 0, priority: 0, pose: actor.frame });
                indices.insert(actor.entity.clone(), index);
                index
            };
            let declaration = self.types.declaration(self.factories[5], actor_index, actor.scheduling.clone())?;
            let handle = ComponentFactory { declaration: &declaration, pool: &mut self.usables, registry,
                grow_one_at_a_time: false }.create(&mut self.entities[index].components,
                    declaration.scheduling.starts_on_spawn(),
                    |_, _, _| Ok(UsableComponent { owner: Default::default(), actor: actor_index }),
                    |_, _, _| Ok(()))?.ok_or("usable component creation disabled")?;
            self.declarations.insert(handle.packed(), declaration);
        }
        for binding in &interactions.readable {
            let definition = definitions.get(binding.entity.as_str()).ok_or("readable entity declaration absent")?;
            let index = if let Some(&index) = indices.get(&binding.entity) { index } else {
                let class = definition.fields.iter().find_map(|(key, value)| match (key.as_str(), value) {
                    ("class", entities::Value::Atom(value)) => Some(value.trim_matches('"')),
                    _ => None,
                }).ok_or("readable entity class absent")?;
                let class = *names.get(class.as_bytes()).ok_or("readable entity class is not registered")?;
                let index = self.entities.len();
                let shared = live.allocate_managed(Arc::from(binding.entity.as_str()), class, None, 0, constructor_tag)?;
                self.entities.push(PhysicsEntity { name: binding.entity.clone(), shared,
                    components: EntityComponents::new(index), class, environment: None, flags: 0,
                    entity_flags: 0x001e_8008, entity_flags_d7: 0, priority: 0,
                    pose: entities::spawn_frame(definition)? });
                indices.insert(binding.entity.clone(), index);
                index
            };
            let item = interactions.acquire_readable_item(&binding.item)?;
            let declaration = self.types.declaration(self.factories[6], Arc::as_ptr(&binding.component) as usize,
                binding.component.scheduling.clone())?;
            let handle = ComponentFactory { declaration: &declaration, pool: &mut self.pickables,
                registry, grow_one_at_a_time: false }.create(&mut self.entities[index].components,
                    declaration.scheduling.starts_on_spawn(), |_, _, _| {
                        let mut component = crate::gameplay::pickable_component::PickableComponent::new(binding.component.clone());
                        component.item = Some(item);
                        Ok(component)
                    }, |_, _, _| Ok(()))?.ok_or("Pickable component creation disabled")?;
            let generation = self.pickables.get(handle.slot).ok_or("new Pickable component absent")?.owner.generation;
            self.readable_components.insert(binding.entity.clone(),
                crate::gameplay::pickable_component::CheckedComponent { handle, generation });
            self.declarations.insert(handle.packed(), declaration);
            if let Some(definition) = &binding.loot {
                let declaration = self.types.declaration(self.factories[7], Arc::as_ptr(definition) as usize,
                    definition.scheduling.clone())?;
                let handle = ComponentFactory { declaration: &declaration, pool: &mut self.loots,
                    registry, grow_one_at_a_time: false }.create(&mut self.entities[index].components,
                        declaration.scheduling.starts_on_spawn(), |_, _, _| {
                            Ok(crate::gameplay::loot_component::EmptyLootComponent {
                                owner: Default::default(), definition: definition.clone(), enabled: true })
                        }, |_, _, _| Ok(()))?.ok_or("Loot component creation disabled")?;
                self.declarations.insert(handle.packed(), declaration);
            }
        }
        Ok(())
    }

    pub(in crate::app) fn readable_identity(&self, entity: &str,
        live: &LiveEntities) -> Result<Option<(crate::gameplay::entity_lifetime::EntityId,
            crate::gameplay::pickable_component::CheckedComponent)>, String> {
        let Some(&reference) = self.readable_components.get(entity) else { return Ok(None); };
        let owner = self.owner(reference.handle)?;
        if owner.generation != reference.generation { return Err("readable component generation is stale".into()); }
        let entity = self.entities.get(owner.entity.ok_or("readable component is unlinked")?)
            .ok_or("readable entity slot is absent")?;
        if live.stage(entity.shared) != Some(crate::gameplay::entity_lifetime::Stage::Spawned)
            || owner.flags & 0xf != 0xa { return Ok(None); }
        Ok(Some((entity.shared, reference)))
    }

    pub(in crate::app) fn readable_item(&self, entity: &str, live: &LiveEntities)
        -> Result<Option<((crate::gameplay::entity_lifetime::EntityId,
            crate::gameplay::pickable_component::CheckedComponent),
            Arc<crate::assets::inventory_item::PickableItem>)>, String> {
        let Some(identity) = self.readable_identity(entity, live)? else { return Ok(None); };
        let item = self.pickables.get(identity.1.handle.slot).and_then(|component| component.item.as_ref())
            .ok_or("readable Pickable item lease is absent")?;
        Ok(Some((identity, item.clone())))
    }
}
