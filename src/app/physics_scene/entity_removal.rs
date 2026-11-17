//! Actual entity headers/component lists for8791f0 and65afa0. The existing
//! lifetime Host owns Stop/Deinitialize callbacks and their pending receipts.
use super::*;
use crate::gameplay::{
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
};
impl PhysicsScene {
    /// Metadata certificates retain only class presence, not parsed source
    /// trees. Inherited source chains are resolved once per distinct resource.
    pub(in crate::app) fn capture_removal_components(
        &mut self,
        catalog: &archive::Catalog,
        document: &entities::Document,
    ) -> Result<(), String> {
        use entities::components::ComponentList;
        let declarations = document
            .definitions
            .iter()
            .filter(|definition| definition.kind == "component")
            .map(|definition| (definition.id.as_str(), definition))
            .collect::<HashMap<_, _>>();
        //014850 skips entityDef parent lookup when the authored idStr is
        //empty;022fd0 makes the same length check before generic inheritance.
        //An explicit malformed value or a nonempty missing path is not that
        //absence case and must remain an acquisition error.
        fn parent(fields:&[(String,entities::Value)])->Result<Option<&str>,String>{
            match entities::field(fields,"inherit"){
                None=>Ok(None),
                Some(entities::Value::Atom(value))=>Ok(Some(value.trim_matches('"')).filter(|name|!name.is_empty())),
                Some(_)=>Err("removal metadata entity inheritance is not a string".into()),
            }
        }
        fn inherited(
            catalog: &archive::Catalog,
            name: &str,
            cache: &mut HashMap<String, Arc<ComponentList>>,
            stack: &mut Vec<String>,
        ) -> Result<Arc<ComponentList>, String> {
            let path = format!("generated/decls/entitydef/{name}.decl");
            let key = path.replace('\\', "/").to_ascii_lowercase();
            if let Some(slots) = cache.get(&key) {
                return Ok(slots.clone());
            }
            if stack.contains(&key) {
                return Err(format!("removal metadata entity inheritance cycle: {path}"));
            }
            stack.push(key.clone());
            let source = crate::assets::declaration_texts::read_common_or_catalog(catalog, &path)?
                .ok_or_else(|| format!("removal metadata source absent: {path}"))?;
            let fields =
                entities::parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
            let mut slots = if ComponentList::expands(&fields).map_err(|error|format!("{path}: {error}"))? {
                if let Some(parent) = parent(&fields).map_err(|error|format!("{path}: {error}"))? {
                    (*inherited(catalog, parent, cache, stack)?).clone()
                } else { ComponentList::default() }
            } else {
                ComponentList::default()
            };
            slots.apply(&fields,true,&path)?;
            stack.pop();
            let slots = Arc::new(slots);
            cache.insert(key, slots.clone());
            Ok(slots)
        }
        self.removal_tampering.resize(self.entities.len(), None);
        let definitions = document
            .definitions
            .iter()
            .filter(|definition| definition.kind == "entity")
            .map(|definition| (definition.id.as_str(), definition))
            .collect::<HashMap<_, _>>();
        let mut cache = HashMap::new();
        for (index, entity) in self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| !entity.retired && !entity.external)
        {
            if self.removal_tampering[index].is_some() { continue; }
            let Some(definition) = definitions.get(entity.name.as_str()) else {
                continue;
            };
            let context=format!("removal metadata entity={}",entity.name);
            let result=(||{
                let mut slots=if ComponentList::expands(&definition.fields)?{
                    if let Some(parent)=parent(&definition.fields)?{
                        (*inherited(catalog,parent,&mut cache,&mut Vec::new())?).clone()
                    }else{ComponentList::default()}
                }else{ComponentList::default()};
                slots.apply(&definition.fields,true,&context)?;
                let mut tampering=false;
                for(slot,fields)in slots.into_components(&context)?{
                    let class=entities::components::entry_class(&fields,&declarations)
                        .map_err(|error|format!("{context}: slot={slot} value={fields:?}: {error}"))?
                        .ok_or_else(||format!("{context}: slot={slot} has no declaration class value={fields:?}"))?;
                    tampering|=class.eq_ignore_ascii_case("cpntAITampering");
                }
                Ok(tampering)
            })();
            self.removal_tampering[index]=Some(result.map_err(|error:String|format!("{context}: {error}"))?);
        }
        Ok(())
    }
    /// Runtime Definition rejects unhosted AITampering before admission, so
    /// its complete typed component list proves absence for this exact slot.
    pub(in crate::app) fn certify_runtime_removal_absence(&mut self, index: usize) {
        self.removal_tampering.resize(self.entities.len(), None);
        self.removal_tampering[index] = Some(false);
    }
    ///External Player/equipment hosts supply the same complete source-list
    ///presence projection while retaining their actual typed component owners.
    pub(in crate::app) fn publish_external_tampering_presence(
        &mut self, live:&LiveEntities, id:EntityId, declared:bool,
    )->Result<(),String>{
        let (_,class)=live.get(id).ok_or("external tampering projection generation is stale")?;
        let index=*self.shared_indices.get(&id).ok_or("external tampering projection Scene owner absent")?;
        let entity=self.entities.get(index).ok_or("external tampering projection header absent")?;
        if entity.retired || !entity.external || entity.shared!=id || entity.class!=class {
            return Err("external tampering projection has a different actual owner".into());
        }
        self.removal_tampering.resize(self.entities.len(),None);
        self.removal_tampering[index]=Some(declared);
        Ok(())
    }
    ///876ff0 does nothing when lookup877720(mode2) finds actual AITampering;
    ///otherwise it invokes Remove. Source-declared but unhosted is an error,
    ///never a fabricated absent component or replacement ghost effect.
    pub(in crate::app) fn remove_or_ghost_removes(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<bool, String> {
        let index = self.removal_index(live, id)?;
        match self.removal_tampering.get(index).copied().flatten() {
            Some(false)=>Ok(true),
            Some(true)=>Err("removeOrGhost requires the declared actual AITampering component lifecycle and context owner".into()),
            None=>Err("removeOrGhost has no complete source component-absence certificate".into()),
        }
    }
    fn removal_index(&self, live: &LiveEntities, id: EntityId) -> Result<usize, String> {
        if live.get(id).is_none() {
            return Err("entity removal generation is stale".into());
        }
        let index = *self
            .shared_indices
            .get(&id)
            .ok_or("entity removal has no actual scene header")?;
        if self.entities[index].retired || self.entities[index].shared != id {
            return Err("entity removal header is stale".into());
        }
        if self.entities[index].external {
            return Err("external Player removal requires its actual player lifetime owner".into());
        }
        Ok(index)
    }
    pub(in crate::app) fn removal_runtime_flags(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<u8, String> {
        let index = self.removal_index(live, id)?;
        self.entity_flags_d4
            .get(index)
            .copied()
            .ok_or_else(|| "entity removal runtime flags absent".into())
    }
    pub(in crate::app) fn removal_component_count(
        &self,
        live: &LiveEntities,
        id: EntityId,
    ) -> Result<usize, String> {
        Ok(self.entities[self.removal_index(live, id)?]
            .components
            .entries
            .values()
            .len())
    }
    pub(in crate::app) fn removal_component_at(
        &self,
        live: &LiveEntities,
        id: EntityId,
        index: usize,
    ) -> Result<(ComponentHandle, u32), String> {
        let entity = self.removal_index(live, id)?;
        let packed = *self.entities[entity]
            .components
            .entries
            .values()
            .get(index)
            .ok_or("entity removal component index is absent")?;
        let handle = ComponentHandle::from_packed(packed)?;
        let owner = self.owner(handle)?;
        if owner.entity != Some(entity) {
            return Err("entity removal component belongs to another actual owner".into());
        }
        Ok((handle, owner.generation))
    }
    pub(in crate::app) fn request_removal_component(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        entity: EntityId,
        handle: ComponentHandle,
        generation: u32,
        state: &mut super::lifetime::Retirement,
    ) -> Result<(), String> {
        let index = self.removal_index(live, entity)?;
        let owner = self.owner(handle)?;
        if owner.entity != Some(index) || owner.generation != generation {
            return Err("entity removal component generation differs".into());
        }
        super::lifetime::request_component_release(
            world, self, live, events, handle, generation, state,
        )
    }
    pub(in crate::app) fn finish_requested_removal(
        &mut self,
        live: &mut LiveEntities,
        entity: EntityId,
        state: &mut super::lifetime::Retirement,
    ) -> Result<(), String> {
        let index = self.removal_index(live, entity)?;
        super::lifetime::finish_requested_entity(self, live, index, state)
    }
    pub(in crate::app) fn detach_removal_child(
        &mut self,
        live: &mut LiveEntities,
        events: &mut EntityEvents,
        parent: EntityId,
        child: EntityId,
    ) -> Result<(), String> {
        if live.parent(child)? != Some(parent) {
            return Err("entity removal child has a different actual parent".into());
        }
        self.removal_index(live, child)?;
        self.unbind_entity(live, child, events)?;
        if live.parent(child)?.is_some() {
            return Err(
                "entity removal child has no admitted reference-frame detach callback".into(),
            );
        }
        Ok(())
    }
}
