//! arkDeclCpntBreakable7ec6d0 and reflected step/chunk/FX records.
//! Canonical component inheritance owns overlays and list resize semantics.
use crate::gameplay::reference_frame::ReferenceFrame;
use crate::{
    archive::Catalog,
    entities::{self, Value},
    gameplay::component_state::declaration::DeclarationScheduling,
};
use entities::scene_motion::{atom, child};
use entities::{field, scope};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Debug)]
pub struct Fx {
    pub declaration: Option<String>,
    pub local: ReferenceFrame,
    pub attached: bool,
}
#[derive(Clone, Debug)]
pub struct Chunk {
    pub entity: Option<String>,
    pub fx: Fx,
    pub remove_after: f32,
    pub keep_bind_info: bool,
}
#[derive(Clone, Debug)]
pub struct Step {
    pub health_threshold: f32,
    pub owner_fx: Fx,
    pub chunks: Vec<Chunk>,
    pub sound_occlusion: f32,
    pub hide_and_detach: bool,
    pub remove_entity: bool,
}
#[derive(Debug)]
pub struct Definition {
    pub steps: Vec<Step>,
    pub impulse_on_chunks: f32,
    pub impulse_falloff: f32,
    pub allow_throw_hits: bool,
}
pub struct Binding {
    pub entity_id: String,
    pub slot: String,
    pub definition: Arc<Definition>,
    pub scheduling: DeclarationScheduling,
}
impl Default for Fx {
    fn default() -> Self {
        Self {
            declaration: None,
            local: ReferenceFrame::default(),
            attached: false,
        }
    }
}
impl Default for Chunk {
    fn default() -> Self {
        Self {
            entity: None,
            fx: Fx::default(),
            remove_after: -1.0,
            keep_bind_info: false,
        }
    }
}
impl Default for Step {
    fn default() -> Self {
        Self {
            health_threshold: 0.0,
            owner_fx: Fx::default(),
            chunks: Vec::new(),
            sound_occlusion: 1.0,
            hide_and_detach: true,
            remove_entity: true,
        }
    }
}
fn reference(fields: &[(String, Value)], name: &str) -> Option<String> {
    atom(fields, name)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("NULL"))
        .map(str::to_owned)
}
fn number(fields: &[(String, Value)], name: &str, default: f32) -> Result<f32, String> {
    crate::gameplay::damage::number(fields, name, default)
}
fn flag(fields: &[(String, Value)], name: &str, default: bool) -> Result<bool, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(v) => Err(format!("invalid breakable {name}: {v}")),
    }
}
fn vector(fields: &[(String, Value)], default: [f32; 3]) -> Result<[f32; 3], String> {
    Ok([
        number(fields, "x", default[0])?,
        number(fields, "y", default[1])?,
        number(fields, "z", default[2])?,
    ])
}
impl Fx {
    fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut local = ReferenceFrame::default();
        local.position = vector(child(fields, "m_localOrigin"), local.position)?;
        let matrix = child(child(fields, "m_localAxis"), "mat");
        for (i, row) in local.basis.iter_mut().enumerate() {
            *row = vector(child(matrix, &format!("mat[{i}]")), *row)?;
        }
        Ok(Self {
            declaration: reference(fields, "m_declFX"),
            local,
            attached: flag(fields, "m_attached", false)?,
        })
    }
}
/// Reflected idList resizing constructs every row first (9d8f10/9d9070),
/// including omitted rows. Sparse authored rows do not disappear.
fn rows(fields: &[(String, Value)]) -> Result<Vec<&[(String, Value)]>, String> {
    let count = atom(fields, "num")
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "invalid breakable list length")?;
    let mut rows = vec![&[][..]; count];
    for (key, value) in fields {
        if let Some(index) = key.strip_prefix("item[").and_then(|s| s.strip_suffix(']')) {
            let index = index
                .parse::<usize>()
                .map_err(|_| "invalid breakable row index")?;
            let row = rows
                .get_mut(index)
                .ok_or("breakable row outside reflected list length")?;
            *row = scope(Some(value)).ok_or("breakable row is not a scope")?;
        }
    }
    Ok(rows)
}
impl Definition {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let steps = rows(child(fields, "m_steps"))?
            .into_iter()
            .map(|fields| {
                let chunks = rows(child(fields, "m_chunks"))?
                    .into_iter()
                    .map(|fields| {
                        Ok(Chunk {
                            entity: reference(fields, "m_entityDef"),
                            fx: Fx::read(child(fields, "m_fx"))?,
                            remove_after: number(fields, "m_removeAfterDelay", -1.)?,
                            keep_bind_info: flag(fields, "m_keepBindInfo", false)?,
                        })
                    })
                    .collect::<Result<_, String>>()?;
                Ok(Step {
                    health_threshold: number(fields, "m_healthThreshold", 0.)?,
                    owner_fx: Fx::read(child(fields, "m_ownerFX"))?,
                    chunks,
                    sound_occlusion: number(fields, "m_soundOcclusionStrength", 1.)?,
                    hide_and_detach: flag(fields, "m_hideAndDetachEntity", true)?,
                    remove_entity: flag(fields, "m_removeEntity", true)?,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            steps,
            impulse_on_chunks: number(fields, "m_impulseOnChunks", 3.)?,
            impulse_falloff: number(fields, "m_impulseFalloff", 2.)?,
            allow_throw_hits: flag(fields, "m_allowThrowHits", false)?,
        })
    }
    pub fn fx_paths(&self) -> impl Iterator<Item = &str> {
        self.steps
            .iter()
            .flat_map(|step| {
                step.owner_fx.declaration.iter().chain(
                    step.chunks
                        .iter()
                        .filter_map(|chunk| chunk.fx.declaration.as_ref()),
                )
            })
            .map(String::as_str)
    }
    /// Ordered preconstruction requests. Equal declarations still have
    /// separate actor generations; immutable resource owners may share payloads.
    pub fn chunks(&self) -> impl Iterator<Item = (usize, usize, &str)> {
        self.steps
            .iter()
            .enumerate()
            .flat_map(|(step, definition)| {
                definition
                    .chunks
                    .iter()
                    .enumerate()
                    .filter_map(move |(chunk, definition)| {
                        definition.entity.as_deref().map(|path| (step, chunk, path))
                    })
            })
    }
}
pub fn bindings(catalog: &Catalog, document: &entities::Document) -> Result<Vec<Binding>, String> {
    let declarations = document
        .definitions
        .iter()
        .filter(|d| d.kind == "component")
        .map(|d| (d.id.as_str(), d))
        .collect::<HashMap<_, _>>();
    let mut cache = HashMap::new();
    let mut result = Vec::new();
    let mut definitions = HashMap::<String, Arc<Definition>>::new();
    let mut read =
        |path: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog, path);
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        for (slot, value) in child(child(&entity.fields, "edit"), "m_componentDecls")
            .iter()
            .filter(|(slot, _)| slot.starts_with("item_add["))
        {
            let Some(entry) = scope(Some(value)) else {
                continue;
            };
            let Some(fields) = entities::components::entry_fields(
                entry,
                "cpntbreakable",
                &declarations,
                &mut cache,
                &mut read,
            )?
            else {
                continue;
            };
            // Only immutable declaration sharing; inline state stays entity scoped.
            let key = if let Some(name) = atom(entry, "mapComponentDecl") {
                format!("map:{name}")
            } else if field(entry, "inlineDecl").is_none() {
                format!("decl:{}", atom(entry, "inheritedDecl").unwrap_or(""))
            } else {
                format!("{}:{slot}", entity.id)
            };
            let definition = if let Some(def) = definitions.get(&key) {
                def.clone()
            } else {
                let def = Arc::new(Definition::read(&fields)?);
                definitions.insert(key, def.clone());
                def
            };
            result.push(Binding {
                entity_id: entity.id.clone(),
                slot: slot.clone(),
                definition,
                scheduling: entities::components::scheduling(
                    &fields,
                    DeclarationScheduling::from_base_flags(0),
                )?,
            });
        }
    }
    Ok(result)
}
