//! One typed component declaration decoder for entity construction87a4b0
//! and Projectile627f60's same-entity component attachments. Generic syntax
//! exists only during acquisition; the real factory and resource owners are
//! retained by Component's typed payload and declaration scheduling.
use super::{Component, Kind};
use crate::{
    archive::Catalog,
    assets::{self, entities},
    gameplay::component_state::{declaration::DeclarationScheduling, types::ComponentTypes},
};
use entities::scene_motion::{atom, child};
use std::{collections::HashMap, sync::Arc};

pub(crate) struct Resolved {
    pub component: Component,
    pub animation: Option<String>,
}
pub(crate) struct Acquired {
    pub decoded: Resolved,
    pub source_paths: Vec<String>,
}
pub(super) fn from_resolved(
    catalog: &Catalog,
    types: &ComponentTypes,
    entity: &str,
    slot: &str,
    declaration: &str,
    inline: bool,
    kind: &str,
    fields: &[(String, entities::Value)],
    read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Resolved, String> {
    let result = (|| {
    let mut animation = None;
    let mut defaults = DeclarationScheduling::from_base_flags(0);
    if kind == "cpntaudio" {
        defaults.add_updates(0x10);
    }
    //57ca40 adds phase4, which clears spawn-start. Notification56d240
    //then removes phase4 via00cd80, restoring the common mask0/flag4.
    if matches!(kind,"cpntaicrewcollectorarea"|"cpntaicrewcollectorcombat"|"cpntaicrewcollectorsearch"|"cpntaicrewcollectorherd") {
        defaults.add_updates(4);
    }
    if kind=="cpntaisearchspotdynamic" { defaults.add_updates(1); }
    //7553d0 requests phase2. The empty Retargeting roster still has a real
    //component owner, optional checked dependencies and normal lifecycle.
    if kind=="cpntretargeting" { defaults.add_updates(2); }
    //4b1650 adds phase0x10 to the code-created FX declaration.
    if kind=="cpntfx" { defaults.add_updates(0x10); }
    let scheduling = Arc::new(entities::components::scheduling(&fields, defaults)?);
    let (native, payload) = match kind {
        "cpntfx" => ("arkCpntFX", Kind::Fx(Arc::new(assets::fx_component::Definition::read(fields)?))),
        "cpntconstraint" => ("arkCpntConstraint", Kind::Constraint(Arc::new(assets::constraints::Definition::read(fields)?))),
        "cpntopenable" => ("arkCpntOpenable",Kind::Openable(Arc::new(assets::openable::Definition::read(catalog,fields)?))),
        "cpntretargeting" => ("arkCpntRetargeting",Kind::Retargeting),
        "cpntflashsurface" => ("arkCpntFlashSurface",Kind::FlashSurface(Arc::new(assets::flash_surface::Definition::read(fields)?))),
        "cpntgrenadecooking" => ("arkCpntGrenadeCooking",Kind::GrenadeCooking(Arc::new(assets::flash_surface::Definition::read(fields)?))),
        "cpntinteractivemap" => ("arkCpntInteractiveMap",Kind::InteractiveMap(Arc::new(assets::flash_surface::MapDefinition::read(fields)?))),
        "cpntrigidbody" => (
            "arkCpntRigidBody",
            Kind::Rigid(Arc::new(entities::rigid_body::Definition {
                damage: entities::rigid_body::contact_damage(catalog, &fields)?,
                settings: entities::rigid_body::from_resolved(entity, &declaration, &fields)?,
                declaration_flags: entities::rigid_body::declaration_flags(&fields)?,
                scheduling: scheduling.clone(),
            })),
        ),
        //Installed entityDef references AND map-local declarations use
        //cpntRigidBodyThrowable; cpntThrowable is the alternate spelling.
        "cpntrigidbodythrowable" | "cpntthrowable" => {
            let value = assets::throwable::Definition::from_resolved(
                catalog,
                entity,
                &declaration,
                &fields,
                scheduling.clone(),
            )?;
            ("arkCpntRigidBodyThrowable", Kind::Throwable(value))
        }
        "cpntprojectile" => {
            let value = assets::projectiles::Declaration::from_resolved(catalog, entity, &fields)?;
            ("arkCpntProjectile", Kind::Projectile(value))
        }
        "cpntbreakable" => {
            let value = Arc::new(assets::breakable::Definition::read(&fields)?);
            ("arkCpntBreakable", Kind::Breakable(value))
        }
        "cpntaiinvestigatespot" | "cpntaisearchspotdynamic" => {
            let value=Arc::new(assets::ai_smart_objects::Definition::read(catalog,kind,fields,inline)?);
            let class=std::str::from_utf8(value.factory_name()).map_err(|_|"AISmartObject class name invalid")?;
            (class,Kind::AISmartSpot(value))
        }
        "cpntgrenade" => ("arkCpntGrenade", Kind::Grenade(Arc::new(assets::grenades::Definition::read(fields)?))),
        "cpntaidangerzone" => ("arkCpntAIDangerZone", Kind::AIDangerZone(Arc::new(assets::ai_world::DangerDefinition::read(fields)?))),
        "cpntaicrewcore"|"cpntaicrewknowledgebb"|"cpntaicrewcoordinationbb"|"cpntaicrewdecision"=>{
            let value=Arc::new(assets::ai_world::crew::Definition::read(catalog,kind,fields)?);
            let class=std::str::from_utf8(value.factory_name()).map_err(|_|"Crew class name is not UTF-8")?;
            (class,Kind::Crew(value))
        }
        "cpntaicrewcollectorarea"|"cpntaicrewcollectorcombat"|"cpntaicrewcollectornotification"|"cpntaicrewcollectorsearch"|"cpntaicrewcollectorherd"=>{
            let value=Arc::new(assets::ai_world::crew_collectors::Definition::read(kind,fields)?);
            let class=std::str::from_utf8(value.factory_name()).map_err(|_|"Crew collector class name is not UTF-8")?;
            (class,Kind::CrewCollector(value))
        }
        "cpntaicrewmodulecombatchase"|"cpntaicrewmodulecombatmelee"|"cpntaicrewmodulesearch"|"cpntaicrewmodulespotlink"|"cpntaicrewmodulecombatranged"|"cpntaicrewmodulecombatfake"|"cpntaicrewmodulecombatflank"|"cpntaicrewmodulespothideout"|"cpntaicrewmodulecombattether"|"cpntaicrewmodulecombatalarm"=>{
            let value=Arc::new(assets::ai_world::crew_modules::Definition::read(kind,fields)?);
            let class=std::str::from_utf8(value.factory_name()).map_err(|_|"Crew module class name is not UTF-8")?;
            (class,Kind::CrewModule(value))
        }
        "cpntaistatistics" => ("arkCpntAIStatistics", Kind::AIStatistics),
        "cpnttimersmanager" => ("arkCpntTimersManager", Kind::TimersManager),
        "cpntaimanagercues" | "cpntaicrewdispatch" | "cpntdsm" | "cpntfleemanager"
        | "cpntaiworldcoordinationbb" | "cpntaivisionmanager" => {
            let class = match kind {
                "cpntaimanagercues" => "arkCpntAIManager_Cues",
                "cpntaicrewdispatch" => "arkCpntAICrewDispatch",
                "cpntdsm" => "arkCpntDSM",
                "cpntfleemanager" => "arkCpntFleeManager",
                "cpntaiworldcoordinationbb" => "arkCpntAIWorldCoordinationBB",
                "cpntaivisionmanager" => "arkCpntAIVisionManager",
                _ => unreachable!(),
            };
            (class, Kind::AIStartup(Arc::new(assets::ai_world::startup_managers::Definition::read(class, fields)?)))
        }
        "cpntdamagezone"=>("arkCpntDamageZone",Kind::DamageZone(Arc::new(
            crate::gameplay::damage_zone::Definition::read(catalog,fields)?))),
        "cpntsoundvolumecontrol" => (
            "arkCpntSoundVolumeControl",
            Kind::SoundVolume(Arc::new(
                crate::gameplay::sound::volume_control::Definition::read(fields)?,
            )),
        ),
        "cpnthealth" | "cpnthealthevolved" | "cpnthealthdoor" => {
            let native = match kind {
                "cpnthealth" => "arkCpntHealth",
                "cpnthealthevolved" => "arkCpntHealthEvolved",
                _ => "arkCpntHealthDoor",
            };
            let value = Arc::new(entities::damage::HealthBinding {
                entity_id: entity.to_owned(),
                slot: slot.to_owned(),
                class: kind.to_owned(),
                maximum: crate::gameplay::damage::number(&fields, "m_health", 1.)?,
                asleep_health: crate::gameplay::damage::number(&fields, "m_asleepHealth", 1.)?,
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
                        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                        .map(|name| {
                            assets::wwise::event_resources::acquire_with(catalog, name, &mut *read)
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
                slot: slot.to_owned(),
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
            animation = atom(&fields, "m_animTree")
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned);
            let default_alias = atom(&fields, "m_defaultAnim")
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned);
            ("arkCpntAnimBase", Kind::AnimBasic { default_alias })
        }
        "cpntaiattentionsource" => (
            "arkCpntAIAttentionSource",
            Kind::AttentionSource(Arc::new(assets::ai_cues::AttentionDefinition::read(fields)?)),
        ),
        "cpntwhaleoiltank" => ("arkCpntWhaleOilTank",Kind::WhaleOilTank(Arc::new(assets::whale_oil_tank::Definition::read(fields)?))),
        "cpntaimassist" => (
            "arkCpntAimAssist",
            Kind::Unhosted {
                class: "arkCpntAimAssist",
            },
        ),
        "cpntobservable" => (
            "arkCpntObservable",
            Kind::Observable(Arc::new(assets::ai_cues::ObservableDefinition::read(fields)?)),
        ),
        _ => {
            return Err(format!(
                "{entity}: component {kind} has no typed runtime payload owner"
            ));
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
                .is_some_and(|name| name.eq_ignore_ascii_case(native.as_bytes()))
        })
        .ok_or_else(|| format!("{entity}: installed component factory {native} is absent"))?;
    Ok::<_, String>(Resolved {
        component: Component {
            slot: slot.to_owned(),
            declaration: declaration.to_owned(),
            inline,
            class_index: factory.class,
            factory: factory.pool_index,
            scheduling,
            kind: payload,
        },
        animation,
    })
    })();
    result.map_err(|error| format!("{entity}: component slot {slot}, declaration {declaration}: {error}"))
}

impl Component {
    /// Acquire an authored arkDeclComponent reference through the same
    /// inheritance and typed decoder as the entity's initial component list.
    /// The caller supplies its actual binding slot and retains source_paths
    /// with the canonical prototype scope until component retirement.
    pub(crate) fn acquire_decl(
        catalog: &Catalog,
        types: &ComponentTypes,
        entity: &str,
        slot: &str,
        declaration: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Acquired, String> {
        super::active(cancelled)?;
        let mut texts = HashMap::<String, Option<String>>::new();
        let mut read = |path: &str| {
            super::active(cancelled)?;
            if let Some(text) = texts.get(path) {
                return Ok(text.clone());
            }
            let text = assets::declaration_texts::read_common_or_catalog(catalog, path)?;
            texts.insert(path.to_owned(), text.clone());
            Ok::<_, String>(text)
        };
        let entry = vec![(
            "inheritedDecl".to_owned(),
            entities::Value::Atom(declaration.to_owned()),
        )];
        let declarations = HashMap::new();
        let kind = entities::components::entry_class(&entry, &declarations)?
            .ok_or("attached component declaration has no class suffix")?;
        let declaration_kind = catalog.component_declaration_kind(&kind)?.to_ascii_lowercase();
        let fields = entities::components::entry_fields(
            &entry,
            &declaration_kind,
            &declarations,
            &mut HashMap::new(),
            &mut read,
        )?
        .ok_or("attached component resolution changed class")?;
        let decoded = from_resolved(
            catalog,
            types,
            entity,
            slot,
            declaration,
            false,
            &kind,
            &fields,
            &mut read,
        )?;
        super::validate_capabilities(types, std::iter::once(&decoded.component))?;
        drop(read);
        super::active(cancelled)?;
        Ok(Acquired {
            decoded,
            source_paths: texts.into_keys().collect(),
        })
    }
}
