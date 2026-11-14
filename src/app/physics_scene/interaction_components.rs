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
    pub(super) fn admit_usable_components(&mut self, document: &entities::Document,
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
                    priority: 0, pose: actor.frame });
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
        Ok(())
    }
}
