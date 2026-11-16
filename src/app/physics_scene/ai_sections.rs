//! Component-less WorldSectionVolume actors retain their actual class/pose.
//! 92c140 ->785460 constructs only idEntity and two empty Trigger rosters;
//! their shared managed roster owner remains lazy until a real touch.
use super::*;

impl PhysicsScene {
    pub(in crate::app) fn admit_ai_section_owners(&mut self, catalog: &archive::Catalog,
        document: &entities::Document, live: &mut crate::gameplay::entity_lifetime::LiveEntities) -> Result<(), String> {
        let mut changed = false;
        for definition in document.definitions.iter().filter(|definition| definition.kind == "entity"
            && entities::scene_motion::atom(&definition.fields, "class") == Some("arkWorldSectionVolume")) {
            if live.find(&definition.id).is_some() { continue; }
            let class = self.types.classes().hierarchy().registration_order().iter().copied().find(|&index|
                self.types.classes().metadata(index).and_then(|metadata|self.types.classes().name_text(metadata.name))
                    == Some(b"arkWorldSectionVolume"))
                .ok_or("WorldSectionVolume class metadata absent")?;
            //An authored component must go through its actual typed factory.
            //The shipped section actors explicitly remove their inherited
            //Trigger component; this is a real component-less class branch.
            let mut chain = if let Some(parent) = entities::scene_motion::atom(&definition.fields, "inherit")
                .filter(|parent| !parent.is_empty() && !parent.eq_ignore_ascii_case("NULL")) {
                entities::player::entity_chain(parent, &mut |path|crate::assets::declaration_texts::read_common_or_catalog(catalog,path))?
            } else { Vec::new() };
            chain.push(definition.fields.clone());
            if !entities::player::component_entries(&chain)?.is_empty() {
                return Err(format!("{}: section owner with actual components was not admitted by their source factories",definition.id));
            }
            let pose = entities::spawn_frame(definition)?;
            let model = entities::model_descriptor(definition, |path|
                crate::assets::declaration_texts::read_common_or_catalog(catalog,path))?;
            let scale = model.as_ref().map_or([1.;3], |model|model.scale);
            let id = self.allocate_entity_header(live, &definition.id, class, 0,
                super::super::runtime_entities::SpawnOptions { frame:pose, scale:None, hidden_disabled:false },
                scale, Some(Arc::from(definition.id.as_str())), false, false)?;
            let index = self.shared_indices[&id];
            if entities::entity_hidden(definition) { self.entities[index].entity_flags |= 1; }
            live.initialized(id)?;
            live.initialize_entity_context(id, self.entities[index].components.phase)?;
            live.spawned(id)?;
            live.publish_spawn_tag(id, "MAIN")?;
            changed = true;
        }
        if changed { self.capture_removal_components(catalog, document)?; }
        Ok(())
    }
}
