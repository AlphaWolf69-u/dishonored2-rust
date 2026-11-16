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

pub(crate) struct Component {
    pub slot: String,
    pub declaration: String,
    pub class_index: usize,
    pub factory: u32,
    pub scheduling: Arc<DeclarationScheduling>,
    pub kind: Kind,
}
pub(crate) enum Kind {
    Rigid(Arc<super::entities::rigid_body::Definition>),
    Throwable(Arc<super::throwable::Definition>),
    Projectile(Arc<super::projectiles::Declaration>),
    Breakable(Arc<super::breakable::Definition>),
    Health(Arc<super::entities::damage::HealthBinding>),
    Audio(Arc<super::entities::audio::AudioBinding>),
    Pickable(Arc<crate::gameplay::pickable_component::PickableDefinition>),
    /// Payload-free reference-frame actor is the real factory32 component.
    ReferenceFrame,
    /// Actual AnimBasic metadata and Clips live with the model's BasicRig.
    AnimBasic,
    /// Source-proven optional manager consumers have no actual runtime owner
    /// in this client. Their metadata is retained, never a dummy component.
    Unhosted { class: &'static str },
}
pub(crate) struct Chunk {
    pub step: usize,
    pub chunk: usize,
    pub definition: Arc<Definition>,
}
pub(crate) struct Definition {
    pub entity: String,
    pub class: String,
    pub class_index: usize,
    pub types: Arc<ComponentTypes>,
    pub model: Option<Arc<super::equipment::attachment::Descriptor>>,
    pub components: Vec<Component>,
    pub projectile: Option<Arc<super::projectiles::Declaration>>,
    pub throwable: Option<Arc<super::throwable::Definition>>,
    pub breakable: Option<Arc<super::breakable::Definition>>,
    pub chunks: Vec<Chunk>,
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
fn validate_capabilities(types:&ComponentTypes,components:&[Component])->Result<(),String>{
    let hierarchy=types.classes().hierarchy();
    for optional in components.iter().filter(|component|matches!(component.kind,Kind::Unhosted{..})){
        for supported in components.iter().filter(|component|!matches!(component.kind,Kind::Unhosted{..})){
            for dependency in &types.definition(supported.factory)?.initialization{
                let range=hierarchy.type_range(dependency.class)?;
                let target=hierarchy.type_id(optional.class_index)?;
                if (dependency.exact&&dependency.class==optional.class_index)||(!dependency.exact&&range[0]<=target&&target<=range[1]){
                    return Err(format!("supported component {} requires unhosted manager component {}: {}",supported.slot,optional.slot,dependency.reason));
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
            let existing=registry.live_values().find(|value| value.entity == entity);
            if let Some(value) = existing {
                return Ok(value);
            }
        }
        stack.push(source.clone());
        let result = (|| {
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
            let entries = entities::player::component_entries(&chain);
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
                let fields = entities::components::entry_fields(
                    &entry,
                    &kind,
                    &declarations,
                    &mut cache,
                    &mut read,
                )?
                .ok_or("runtime component resolution changed class")?;
                let declaration = atom(&entry, "inheritedDecl")
                    .or_else(|| atom(&entry, "mapComponentDecl"))
                    .unwrap_or("")
                    .to_owned();
                let mut defaults = DeclarationScheduling::from_base_flags(0);
                if kind == "cpntaudio" {
                    defaults.add_updates(0x10);
                }
                let scheduling = Arc::new(entities::components::scheduling(&fields, defaults)?);
                let (native, payload) = match kind.as_str() {
                    "cpntrigidbody" => (
                        "arkCpntRigidBody",
                        Kind::Rigid(Arc::new(entities::rigid_body::Definition {
                            settings: entities::rigid_body::from_resolved(
                                entity,
                                &declaration,
                                &fields,
                            )?,
                            declaration_flags: entities::rigid_body::declaration_flags(&fields)?,
                            scheduling: scheduling.clone(),
                        })),
                    ),
                    "cpntrigidbodythrowable" => {
                        let value = super::throwable::Definition::from_resolved(
                            catalog,
                            entity,
                            &declaration,
                            &fields,
                            scheduling.clone(),
                        )?;
                        throwable = Some(value.clone());
                        ("arkCpntRigidBodyThrowable", Kind::Throwable(value))
                    }
                    "cpntprojectile" => {
                        let value = super::projectiles::Declaration::from_resolved(
                            catalog, entity, &fields,
                        )?;
                        projectile = Some(value.clone());
                        ("arkCpntProjectile", Kind::Projectile(value))
                    }
                    "cpntbreakable" => {
                        let value = Arc::new(super::breakable::Definition::read(&fields)?);
                        breakable = Some(value.clone());
                        ("arkCpntBreakable", Kind::Breakable(value))
                    }
                    "cpnthealth" | "cpnthealthevolved" | "cpnthealthdoor" => {
                        let native = match kind.as_str() {
                            "cpnthealth" => "arkCpntHealth",
                            "cpnthealthevolved" => "arkCpntHealthEvolved",
                            _ => "arkCpntHealthDoor",
                        };
                        let value = Arc::new(entities::damage::HealthBinding {
                            entity_id: entity.to_owned(),
                            slot: format!("item_add[{slot}]"),
                            class: kind.clone(),
                            maximum: crate::gameplay::damage::number(&fields, "m_health", 1.)?,
                            asleep_health: crate::gameplay::damage::number(
                                &fields,
                                "m_asleepHealth",
                                1.,
                            )?,
                            receiver: crate::gameplay::damage::Receiver::read(&fields)?,
                            scheduling: (*scheduling).clone(),
                        });
                        (native, Kind::Health(value))
                    }
                    "cpntaudio" => {
                        let list = child(&fields, "m_startSoundEvents");
                        let count = atom(list, "num")
                            .unwrap_or("0")
                            .parse::<usize>()
                            .map_err(|_| "invalid runtime audio startup count")?;
                        let mut events = Vec::with_capacity(count);
                        for index in 0..count {
                            events.push(
                                atom(list, &format!("item[{index}]"))
                                    .filter(|name| {
                                        !name.is_empty() && !name.eq_ignore_ascii_case("NULL")
                                    })
                                    .map(|name| {
                                        super::wwise::event_resources::acquire_with(
                                            catalog, name, &mut read,
                                        )
                                    })
                                    .transpose()?,
                            );
                        }
                        let propagation = match atom(&fields, "m_enablePropagation") {
                            None | Some("true") => true,
                            Some("false") => false,
                            _ => return Err("invalid runtime audio propagation flag".into()),
                        };
                        let value = Arc::new(entities::audio::AudioBinding {
                            entity_id: entity.to_owned(),
                            slot: format!("item_add[{slot}]"),
                            scheduling: (*scheduling).clone(),
                            settings: crate::gameplay::sound::component::Settings::read(&fields)?,
                            propagation,
                            events,
                            frame: crate::gameplay::reference_frame::ReferenceFrame::default(),
                        });
                        ("arkCpntAudio", Kind::Audio(value))
                    }
                    "cpntpickable_simple" | "cpntpickablesimple" => (
                        "arkCpntPickable_Simple",
                        Kind::Pickable(entities::carrying::definition(catalog, &fields)?),
                    ),
                    "cpntanimbase" => {
                        animation = Some(
                            atom(&fields, "m_animTree")
                                .ok_or("runtime AnimBasic tree is absent")?
                                .to_owned(),
                        );
                        ("arkCpntAnimBase", Kind::AnimBasic)
                    }
                    "cpntaiattentionsource"=>("arkCpntAIAttentionSource",Kind::Unhosted{class:"arkCpntAIAttentionSource"}),
                    "cpntaimassist"=>("arkCpntAimAssist",Kind::Unhosted{class:"arkCpntAimAssist"}),
                    "cpntobservable"=>("arkCpntObservable",Kind::Unhosted{class:"arkCpntObservable"}),
                    _ => {
                        return Err(format!(
                            "{entity}: component {kind} has no typed runtime payload owner"
                        ))
                    }
                };
                let factory = types
                    .factories()
                    .iter()
                    .find(|factory| {
                        types
                            .classes()
                            .metadata(factory.class)
                            .and_then(|data| types.classes().name_text(data.name))
                            .is_some_and(|name|name.eq_ignore_ascii_case(native.as_bytes()))
                    })
                    .ok_or_else(|| {
                        format!("{entity}: installed component factory {native} is absent")
                    })?;
                components.push(Component {
                    slot: format!("item_add[{slot}]"),
                    declaration,
                    class_index: factory.class,
                    factory: factory.pool_index,
                    scheduling,
                    kind: payload,
                });
            }
            validate_capabilities(&types,&components)?;
            let projected = entities::Definition {
                kind: "entity".into(),
                class: class.clone(),
                id: entity.to_owned(),
                fields,
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
            let source_paths = texts.into_keys().collect();
            let mut chunks = Vec::new();
            if let Some(definition) = &breakable {
                for (step, chunk, entity) in definition.chunks() {
                    chunks.push(Chunk {
                        step,
                        chunk,
                        definition: Self::acquire_inner(catalog, &name(entity), cancelled, stack)?,
                    });
                }
            }
            active(cancelled)?;
            Ok(Arc::new(Self {
                entity: entity.to_owned(),
                class,
                class_index,
                types,
                model,
                components,
                projectile,
                throwable,
                breakable,
                chunks,
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
