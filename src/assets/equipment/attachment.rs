//! Selected ammunition's authored attachment entity (44dd00/62b240).
//! The current gun/crossbow attachment defs are ordinary rigid idEntity
//! models; an absent animation component does not imply a synthetic rig.
use super::{SourceRead, check_active};
use crate::assets::{archive::Catalog, bmodel, entities};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

pub(crate) struct Descriptor {
    pub entity: String,
    pub class: String,
    pub model: String,
    pub cast_shadows: bool,
    pub placement: entities::PlacedModel,
    pub geometry: Arc<bmodel::ModelSource>,
}

/// Weak declaration projections only. Geometry acquisition stays with the
/// canonical BML/BTRI owner; this table never pins an unselected ammunition.
#[derive(Default)]
pub(crate) struct Sources {
    catalog: Option<u64>,
    live: HashMap<String, Weak<Descriptor>>,
}

impl Sources {
    pub(crate) fn acquire(
        &mut self,
        catalog: &Catalog,
        entity: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<Descriptor>, String> {
        check_active(cancelled)?;
        if self.catalog.is_some_and(|id| id != catalog.identity()) {
            return Err("ammunition attachment belongs to another catalog".into());
        }
        self.catalog = Some(catalog.identity());
        let key = entity.replace('\\', "/").to_ascii_lowercase();
        if !catalog.has_overlays() {
            if let Some(value) = self.live.get(&key).and_then(Weak::upgrade) {
                return Ok(value);
            }
        }
        self.live.retain(|_, value| value.strong_count() != 0);
        let mut source = SourceRead {
            catalog,
            cancelled,
            texts: HashMap::new(),
        };
        let mut fields = Vec::new();
        for ancestor in entities::player::entity_chain(&key, &mut |path| source.read(path))? {
            entities::overlay_owned(&mut fields, ancestor);
        }
        let class = entities::scene_motion::atom(&fields, "class")
            .ok_or("ammunition attachment class is absent")?
            .to_owned();
        // Current selected attachment declarations use the base constructor
        // and no component factories. Reject a new family before publishing
        // an identity instead of pretending its initialization succeeded.
        if class != "idEntity" {
            return Err(format!(
                "{key}: attachment class {class} has no lifecycle adapter"
            ));
        }
        let edit = entities::scene_motion::child(&fields, "edit");
        let components = entities::scene_motion::child(edit, "m_componentDecls");
        if components.iter().any(|(name, _)| {
            name.starts_with("item_add[")
                || name.starts_with("item_override[")
                || name.starts_with("item[")
        }) {
            return Err(format!(
                "{key}: component-bearing ammunition attachment is unsupported"
            ));
        }
        let graph = entities::scene_motion::child(edit, "m_kiscule");
        if graph.iter().any(|(name, _)| name != "m_version") {
            return Err(format!(
                "{key}: graph-bearing ammunition attachment is unsupported"
            ));
        }
        let model = entities::scene_motion::atom(
            entities::scene_motion::child(edit, "renderModelInfo"),
            "model",
        )
        .ok_or("ammunition attachment has no render model")?
        .to_owned();
        let cast_shadows = match entities::scene_motion::atom(
            entities::scene_motion::child(edit, "renderModelInfo"),
            "m_castShadows",
        ) {
            None | Some("true") => true,
            Some("false") => false,
            Some(value) => return Err(format!("{key}: invalid m_castShadows {value}")),
        };
        if !model.ends_with(".lwo") {
            return Err(format!(
                "{key}: attachment model {model} needs its own animated lifecycle"
            ));
        }
        let definition = entities::Definition {
            kind: "entity".into(),
            class: class.clone(),
            id: key.clone(),
            fields,
        };
        let placement = entities::model_descriptor(&definition, |path| source.read(path))?
            .ok_or("ammunition attachment model descriptor is absent")?;
        check_active(cancelled)?;
        let geometry = bmodel::acquire(
            catalog,
            &placement.resource_path,
            bmodel::CommonModels::shared(catalog),
        )?
        .ok_or_else(|| format!("attachment model missing: {}", placement.resource_path))?;
        check_active(cancelled)?;
        let descriptor = Arc::new(Descriptor {
            entity: key.clone(),
            class,
            model,
            cast_shadows,
            placement,
            geometry: Arc::new(geometry),
        });
        if !catalog.has_overlays() {
            self.live.insert(key, Arc::downgrade(&descriptor));
        }
        Ok(descriptor)
    }
}
