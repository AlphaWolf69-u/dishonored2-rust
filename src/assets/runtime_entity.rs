//! Typed runtime entitydef prototype. Entity inheritance and component slot
//! order follow87a4b0; no generic declaration tree survives acquisition.
use super::entities;
use crate::{
    archive::Catalog,
    gameplay::component_state::{declaration::DeclarationScheduling, types::ComponentTypes},
};
use entities::scene_motion::{atom, child};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
pub(crate) mod component;
mod stick;
mod weapon_variant;
pub(crate) use weapon_variant::weapon_variant;

#[derive(Clone)]
pub(crate) struct Component {
    pub slot: String,
    pub declaration: String,
    pub inline: bool,
    pub class_index: usize,
    pub factory: u32,
    pub scheduling: Arc<DeclarationScheduling>,
    pub kind: Kind,
}
#[derive(Clone)]
pub(crate) enum Kind {
    Rigid(Arc<super::entities::rigid_body::Definition>),
    Throwable(Arc<super::throwable::Definition>),
    Projectile(Arc<super::projectiles::Declaration>),
    Breakable(Arc<super::breakable::Definition>),
    Health(Arc<super::entities::damage::HealthBinding>),
    SoundVolume(Arc<crate::gameplay::sound::volume_control::Definition>),
    DamageZone(Arc<crate::gameplay::damage_zone::Definition>),
    Grenade(Arc<super::grenades::Definition>),
    AIDangerZone(Arc<super::ai_world::DangerDefinition>),
    AIStartup(Arc<super::ai_world::startup_managers::Definition>),
    Crew(Arc<super::ai_world::crew::Definition>),
    CrewCollector(Arc<super::ai_world::crew_collectors::Definition>),
    CrewModule(Arc<super::ai_world::crew_modules::Definition>),
    AIStatistics,
    TimersManager,
    Observable(Arc<super::ai_cues::ObservableDefinition>),
    AttentionSource(Arc<super::ai_cues::AttentionDefinition>),
    Audio(Arc<super::entities::audio::AudioBinding>),
    Pickable(Arc<crate::gameplay::pickable_component::PickableDefinition>),
    /// Payload-free reference-frame actor is the real factory32 component.
    ReferenceFrame,
    /// Actual AnimBasic metadata and Clips live with the model's BasicRig.
    AnimBasic {
        default_alias: Option<String>,
    },
    /// Source-proven optional manager consumers have no actual runtime owner
    /// in this client. Their metadata is retained, never a dummy component.
    Unhosted {
        class: &'static str,
    },
}
#[derive(Clone)]
pub(crate) struct Chunk {
    pub component: usize,
    pub step: usize,
    pub chunk: usize,
    pub definition: Arc<Definition>,
}
#[derive(Clone)]
pub(crate) struct Definition {
    pub entity: String,
    pub class: String,
    pub class_index: usize,
    /// idDynamicEntity's reflected bool at +360 (881d90/8820f0).
    pub hide_on_spawn: bool,
    /// arkBreakable's reflected bool at +360 (4e6540/1417fcc40).
    pub navmesh_static_cutting: bool,
    pub types: Arc<ComponentTypes>,
    pub model: Option<Arc<super::equipment::attachment::Descriptor>>,
    pub components: Vec<Component>,
    pub projectile: Option<Arc<super::projectiles::Declaration>>,
    pub throwable: Option<Arc<super::throwable::Definition>>,
    pub breakable: Option<Arc<super::breakable::Definition>>,
    pub chunks: Vec<Chunk>,
    ///Actual Cues/Crew child prototypes, constructed by their real Init.
    pub startup_children: Vec<Arc<Definition>>,
    pub stick_components: Vec<Component>,
    pub stick_model: Option<Arc<super::equipment::attachment::Descriptor>>,
    pub stick_chunks: Vec<Chunk>,
    /// Exact source read-history ownership, without retaining parsed trees.
    pub source_paths: Vec<String>,
}

fn name(entity: &str) -> String {
    entity
        .replace('\\', "/")
        .to_ascii_lowercase()
        .trim_start_matches("generated/decls/entitydef/")
        .trim_end_matches(".decl")
        .to_owned()
}
fn path(entity: &str) -> String {
    format!("generated/decls/entitydef/{}.decl", name(entity))
}
fn active(cancelled: Option<&dyn Fn() -> bool>) -> Result<(), String> {
    if cancelled.is_some_and(|cancelled| cancelled()) {
        Err("runtime entity acquisition cancelled".into())
    } else {
        Ok(())
    }
}
fn validate_capabilities<'a>(
    types: &ComponentTypes,
    components: impl Iterator<Item = &'a Component> + Clone,
) -> Result<(), String> {
    let hierarchy = types.classes().hierarchy();
    //HealthEvolved has its own unhosted payload and callbacks. HealthDoor uses
    //the actual DoorHealthComponent pool and39edc0 virtual calculation.
    //Reject before resource warming or actor allocation, never substitute
    //the base Health pool merely because the type inherits from it.
    for component in components
        .clone()
        .filter(|component| matches!(component.kind, Kind::Health(_)))
    {
        let class = types
            .classes()
            .metadata(component.class_index)
            .ok_or("runtime Health class metadata absent")?;
        let name = types.classes().name_text(class.name);
        if name != Some(b"arkCpntHealth".as_slice())
            && name != Some(b"arkCpntHealthDoor".as_slice())
        {
            return Err(format!(
                "runtime Health component {} requires its actual derived payload owner",
                component.slot
            ));
        }
    }
    for optional in components
        .clone()
        .filter(|component| matches!(component.kind, Kind::Unhosted { .. }))
    {
        for supported in components
            .clone()
            .filter(|component| !matches!(component.kind, Kind::Unhosted { .. }))
        {
            for dependency in &types.definition(supported.factory)?.initialization {
                let range = hierarchy.type_range(dependency.class)?;
                let target = hierarchy.type_id(optional.class_index)?;
                if (dependency.exact && dependency.class == optional.class_index)
                    || (!dependency.exact && range[0] <= target && target <= range[1])
                {
                    return Err(format!(
                        "supported component {} requires unhosted manager component {}: {}",
                        supported.slot, optional.slot, dependency.reason
                    ));
                }
            }
        }
    }
    Ok(())
}
impl Definition {
    pub(crate) fn acquire(
        catalog: &Catalog,
        entity: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<Self>, String> {
        Self::acquire_inner(catalog, &name(entity), cancelled, &mut Vec::new())
    }
    fn acquire_inner(
        catalog: &Catalog,
        entity: &str,
        cancelled: Option<&dyn Fn() -> bool>,
        stack: &mut Vec<String>,
    ) -> Result<Arc<Self>, String> {
        active(cancelled)?;
        let source = path(entity);
        if stack.iter().any(|item| item == &source) {
            return Err(format!("runtime entity preconstruction cycle at {entity}"));
        }
        if !catalog.has_overlays() {
            let registry = catalog
                .runtime_entity_definitions
                .get_or_init(Default::default)
                .lock()
                .map_err(|_| "entity definition registry is poisoned")?;
            let existing = registry.live_values().find(|value| value.entity == entity);
            if let Some(value) = existing {
                return Ok(value);
            }
        }
        stack.push(source.clone());
        let result: Result<Arc<Self>, String> = (|| {
            let types = ComponentTypes::acquire(catalog)?;
            let mut texts = HashMap::<String, Option<String>>::new();
            let mut read = |path: &str| {
                active(cancelled)?;
                if let Some(text) = texts.get(path) {
                    return Ok(text.clone());
                }
                let text = super::declaration_texts::read_common_or_catalog(catalog, path)?;
                texts.insert(path.to_owned(), text.clone());
                Ok::<_, String>(text)
            };
            let chain = entities::player::entity_chain(entity, &mut read)?;
            let entries = entities::player::component_entries(&chain)?;
            let mut fields = Vec::new();
            for layer in chain {
                entities::overlay_owned(&mut fields, layer);
            }
            let class = atom(&fields, "class")
                .ok_or("runtime entity class is absent")?
                .to_owned();
            let class_index = types
                .classes()
                .hierarchy()
                .registration_order()
                .iter()
                .copied()
                .find(|&index| {
                    types
                        .classes()
                        .metadata(index)
                        .and_then(|data| types.classes().name_text(data.name))
                        == Some(class.as_bytes())
                })
                .ok_or_else(|| {
                    format!("runtime entity class {class} is absent from installed metadata")
                })?;
            let mut components = Vec::new();
            let (mut projectile, mut throwable, mut breakable) = (None, None, None);
            let mut animation = None;
            let declarations = HashMap::new();
            let mut cache = HashMap::new();
            for (slot, entry) in entries {
                active(cancelled)?;
                let kind =
                    entities::components::entry_class(&entry, &declarations)?.ok_or_else(|| {
                        format!("{entity}: component slot{slot} has no declaration class")
                    })?;
                let declaration_kind = catalog.component_declaration_kind(&kind)?.to_ascii_lowercase();
                let fields = entities::components::entry_fields(
                    &entry,
                    &declaration_kind,
                    &declarations,
                    &mut cache,
                    &mut read,
                )?
                .ok_or("runtime component resolution changed class")?;
                let declaration = atom(&entry, "inheritedDecl")
                    .or_else(|| atom(&entry, "mapComponentDecl"))
                    .unwrap_or("")
                    .to_owned();
                let decoded = component::from_resolved(
                    catalog,
                    &types,
                    entity,
                    &format!("item_add[{slot}]"),
                    &declaration,
                    entities::scope(entities::field(&entry, "inlineDecl")).is_some(),
                    &kind,
                    &fields,
                    &mut read,
                )?;
                match &decoded.component.kind {
                    Kind::Projectile(value) => projectile = Some(value.clone()),
                    Kind::Throwable(value) => throwable = Some(value.clone()),
                    Kind::Breakable(value) => breakable = Some(value.clone()),
                    _ => {}
                }
                if matches!(&decoded.component.kind, Kind::AnimBasic { .. }) {
                    animation = decoded.animation;
                }
                components.push(decoded.component);
            }
            validate_capabilities(&types, components.iter())?;
            let projected = entities::Definition {
                kind: "entity".into(),
                class: class.clone(),
                id: entity.to_owned(),
                fields,
            };
            let hide_on_spawn = if class == "idDynamicEntity" {
                match entities::field(child(&projected.fields, "edit"), "hide") {
                    None => false,
                    Some(entities::Value::Atom(value)) if value == "true" => true,
                    Some(entities::Value::Atom(value)) if value == "false" => false,
                    Some(value) => return Err(format!("{entity}: invalid idDynamicEntity.hide {value:?}")),
                }
            } else {
                false
            };
            let navmesh_static_cutting = if class == "arkBreakable" {
                match entities::field(child(&projected.fields, "edit"), "m_navmesh_staticCutting") {
                    None => false,
                    Some(entities::Value::Atom(value)) if value == "true" => true,
                    Some(entities::Value::Atom(value)) if value == "false" => false,
                    Some(value) => return Err(format!("{entity}: invalid arkBreakable.m_navmesh_staticCutting {value:?}")),
                }
            } else {
                false
            };
            let model = atom(
                child(child(&projected.fields, "edit"), "renderModelInfo"),
                "model",
            )
            .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
            .map(|_| {
                super::equipment::attachment::project_world(
                    catalog,
                    &projected,
                    animation.as_deref(),
                    cancelled,
                )
            })
            .transpose()?;
            drop(read);
            let mut source_paths: Vec<_> = texts.into_keys().collect();
            let mut chunks = Vec::new();
            let mut startup_children: Vec<Arc<Definition>> = Vec::new();
            for (component, descriptor) in components.iter().enumerate() {
                if let Kind::AIStartup(manager) = &descriptor.kind {
                    for source in manager.child_sources() {
                        let source = name(source);
                        if !startup_children.iter().any(|child| child.entity == source) {
                            startup_children.push(Self::acquire_inner(catalog, &source, cancelled, stack)?);
                        }
                    }
                }
                if let Kind::Breakable(definition) = &descriptor.kind {
                    for (step, chunk, entity) in definition.chunks() {
                        chunks.push(Chunk {
                            component,
                            step,
                            chunk,
                            definition: Self::acquire_inner(
                                catalog,
                                &name(entity),
                                cancelled,
                                stack,
                            )?,
                        });
                    }
                }
            }
            let stick = stick::prepare(
                catalog,
                entity,
                &class,
                &types,
                projectile.as_deref(),
                model.clone(),
                cancelled,
                stack,
            )?;
            validate_capabilities(&types, components.iter().chain(stick.components.iter()))?;
            source_paths.extend(stick.source_paths);
            active(cancelled)?;
            Ok(Arc::new(Self {
                entity: entity.to_owned(),
                class,
                class_index,
                hide_on_spawn,
                navmesh_static_cutting,
                types,
                model,
                components,
                projectile,
                throwable,
                breakable,
                chunks,
                startup_children,
                stick_components: stick.components,
                stick_model: stick.model,
                stick_chunks: stick.chunks,
                source_paths,
            }))
        })();
        stack.pop();
        let value = result?;
        active(cancelled)?;
        catalog
            .runtime_entity_definitions
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "entity definition registry is poisoned")?
            .acquire_with_sharing(&source, 0, !catalog.has_overlays(), |_| Ok(value))
    }
}

pub(crate) fn register_header(
    catalog: &Catalog,
    entity: &str,
    marker: u8,
    persistent: bool,
) -> Result<(), String> {
    catalog
        .runtime_entity_definitions
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "entity definition registry is poisoned")?
        .register(&path(entity), marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.runtime_entity_definitions.get() {
        owner
            .lock()
            .map_err(|_| "entity definition registry is poisoned")?
            .promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.runtime_entity_definitions.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "entityDef", marker);
        owner.report(catalog.identity(), "entityDef", "scope-released", |value| {
            value.source_paths.iter().map(String::capacity).sum()
        });
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.runtime_entity_definitions.get() {
        owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .report(
                catalog.identity(),
                "entityDef",
                "catalog-retiring",
                |value| value.source_paths.iter().map(String::capacity).sum(),
            );
    }
}
pub(crate) fn source_history(
    registry: &super::resource_registry::Registry<Definition>,
) -> HashSet<String> {
    registry
        .live_values()
        .flat_map(|value| value.source_paths.clone())
        .collect()
}
impl std::fmt::Debug for Definition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityDefinition")
            .field("entity", &self.entity)
            .field("class", &self.class)
            .field("components", &self.components.len())
            .finish_non_exhaustive()
    }
}
