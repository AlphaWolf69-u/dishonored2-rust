//! Inventory-demanded equipment preparation. Entity/component inheritance,
//! rigs, model geometry and compressed clips keep their existing owners.
//! 619380/619640 admit the inventory item's actual equipment entity; 61ac10
//! posts its default alias, 61aed0/61b010 bind it to an arm or the body.
use super::{
    animation_rig::{Clips, Rig},
    archive::Catalog,
    bmd6, entities,
};
use crate::gameplay::player::{
    equipment::{AttachmentBinding, EquipmentBinding, EquipmentInput, EquipmentTree},
    weapons::{Equipment, Ranged, RangedFamily},
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

pub(crate) mod attachment;
mod tree;
mod firing;

/// Immutable selected-entity projection. No live playback state or independent
/// material/geometry cache lives here. The host retains this lease while the
/// inventory entity exists and through its renderer's acknowledgement.
pub struct Metadata {
    pub entity: String,
    pub class_name: String,
    /// Complete inherited component admission, used by6dfa50's mode1 lookup.
    pub ai_tampering_declared: bool,
    pub model: Option<String>,
    pub sound_settings: Option<crate::gameplay::sound::component::Settings>,
    pub component_kind: String,
    pub equipment: Equipment,
    pub ranged: Option<Ranged>,
    pub melee: Option<Arc<crate::gameplay::player::melee::Declaration>>,
    pub heart: Option<Arc<crate::gameplay::player::weapons::heart::Definition>>,
    pub heart_barks: Option<Arc<crate::gameplay::player::weapons::heart::barks::Definition>>,
    pub throwable: Option<Arc<crate::gameplay::player::weapons::throwable::Definition>>,
    pub grenade: Option<Arc<crate::assets::grenades::Definition>>,
    pub grenade_scheduling: Option<Arc<crate::gameplay::component_state::declaration::DeclarationScheduling>>,
}

pub struct Descriptor {
    pub metadata: Arc<Metadata>,
    /// The rendered, animated entity. Equipment declared without a render
    /// model (Emily's spyglass is a bare idEntity with its item component)
    /// has none: 8542b0 casts an equipment entity only when it has an
    /// animation component, so it never gets a tree.
    pub visual: Option<Visual>,
    pub sound_settings: Option<crate::gameplay::sound::component::Settings>,
}

pub struct Visual {
    pub model: String,
    pub rig: Arc<Rig>,
    pub geometry: Arc<bmd6::ModelInfo>,
    pub animation_tree: String,
    pub animations: Vec<crate::edge_animation::EdgeAnimation>,
    pub animation_aliases: HashMap<String, usize>,
    default_clip: Option<usize>,
    ///62ba10 cached model-local bolt socket sampled at the Fire scene key.
    pub projectile_offset: Option<bevy_math::Vec3>,
}

impl std::ops::Deref for Descriptor {
    type Target = Metadata;
    fn deref(&self) -> &Metadata {
        &self.metadata
    }
}

impl Descriptor {
    /// Original slot 1 selects body+370 (left arm); slot 0 selects +680
    /// (right arm), 61aed0. A holstered animated entity binds to the body,
    /// 61ac10/61b010, using the actual unequipped tag.
    pub fn binding(
        &self,
        body: &Rig,
        arms: &[Arc<Rig>; 2],
        equipped: bool,
    ) -> Result<(EquipmentBinding, usize, (bevy_math::Vec3, bevy_math::Mat3)), String> {
        let (owner, binding, tag) = if equipped {
            let side = match self.equipment.slot {
                1 => 0,
                0 => 1,
                slot => return Err(format!("equipment slot {slot} has no player arm binding")),
            };
            (
                &*arms[side],
                EquipmentBinding::Arm(side),
                self.equipment.equipped_bind_tag.as_deref(),
            )
        } else {
            (
                body,
                EquipmentBinding::Body,
                self.equipment.unequipped_bind_tag.as_deref(),
            )
        };
        let tag = tag.ok_or_else(|| {
            format!(
                "{} has no {} bind tag",
                self.entity,
                if equipped { "equipped" } else { "unequipped" }
            )
        })?;
        // 875e80 -> 879890: the parent md6 property group named by the bound
        // entity's model (+0x1bb0), falling back to _info (25bfe0).
        let model = &self.visual.as_ref().ok_or("equipment without a render model has no binding")?.model;
        let (joint, translation, rotation) = owner.tag_quaternion_in_group(model, tag)?;
        Ok((binding, joint, (translation, bevy_math::Mat3::from_quat(rotation))))
    }

    pub fn instantiate(
        &self,
        body: &Rig,
        arms: &[Arc<Rig>; 2],
        equipped: bool,
    ) -> Result<Option<EquipmentTree>, String> {
        let Some(visual) = self.visual.as_ref() else { return Ok(None) };
        let (binding, bind_joint, bind) = self.binding(body, arms, equipped)?;
        let (owner, joint, offset) = self.binding(body, arms, true)?;
        let equipped_attachment = AttachmentBinding {
            owner,
            joint,
            offset,
        };
        let (owner, joint, offset) = self.binding(body, arms, false)?;
        let unequipped_attachment = AttachmentBinding {
            owner,
            joint,
            offset,
        };
        Ok(Some(EquipmentTree {
            managed_entity: None,
            sound_settings: self.sound_settings,
            rig: visual.rig.clone(),
            model: visual.model.clone(),
            animations: visual.animations.clone(),
            animation_aliases: visual.animation_aliases.clone(),
            input: EquipmentInput::DefaultAnimation {
                clip: visual.default_clip,
                time: 0.0,
            },
            binding,
            equipped_attachment,
            unequipped_attachment,
            bind_joint,
            bind,
            visible: equipped || !self.equipment.hide_unequipped,
        }))
    }

    /// Change only the real entity attachment/visibility at the state's
    /// admitted equip/unequip callback. Clip clocks and Midnight state remain.
    pub fn bind(
        &self,
        tree: &mut EquipmentTree,
        body: &Rig,
        arms: &[Arc<Rig>; 2],
        equipped: bool,
    ) -> Result<(), String> {
        let (binding, joint, bind) = self.binding(body, arms, equipped)?;
        tree.binding = binding;
        tree.bind_joint = joint;
        tree.bind = bind;
        tree.visible = equipped || !self.equipment.hide_unequipped;
        Ok(())
    }
}

/// Level-owned weak projections. Live inventory/render leases determine the
/// lifetime; this table does not pin an unequipped arsenal. Editable catalogs
/// create independent generations, as the canonical rig/model owners do.
#[derive(Default)]
pub struct Sources {
    catalog: Option<u64>,
    entities: HashMap<String, Weak<Descriptor>>,
    metadata: HashMap<String, Weak<Metadata>>,
    attachments: attachment::Sources,
}

impl Sources {
    /// 6170b0 ->619380 constructs every actual quantity-positive equipment
    /// entity. A truly model-less entity has no renderer payload; its typed
    /// equipment/component ownership still belongs to the inventory runtime.
    pub fn acquire_owned(
        &mut self,
        catalog: &Catalog,
        entity: &str,
        mut enumeration: impl FnMut(&str, &str) -> Option<i32>,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Option<Arc<Descriptor>>, String> {
        let metadata = self.metadata(catalog, entity, &mut enumeration, cancelled)?;
        if metadata.model.is_none() {
            return Ok(None);
        }
        self.acquire(catalog, entity, enumeration, cancelled)
            .map(Some)
    }
    pub(crate) fn acquire_attachment(
        &mut self,
        catalog: &Catalog,
        entity: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<attachment::Descriptor>, String> {
        self.attachments.acquire(catalog, entity, cancelled)
    }

    pub(crate) fn acquire_upgrade(
        &mut self,
        catalog: &Catalog,
        declaration: &str,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<attachment::Upgrade>, String> {
        self.attachments
            .acquire_upgrade(catalog, declaration, cancelled)
    }

    /// Metadata-only ownership for shortcuts/autoequip arbitration. No model,
    /// rig, tree, animation, material, texture or GPU surface is acquired.
    pub fn metadata(
        &mut self,
        catalog: &Catalog,
        entity: &str,
        mut enumeration: impl FnMut(&str, &str) -> Option<i32>,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<Metadata>, String> {
        check_active(cancelled)?;
        if self.catalog.is_some_and(|id| id != catalog.identity()) {
            return Err("equipment metadata belongs to another level catalog".into());
        }
        self.catalog = Some(catalog.identity());
        let key = entity.replace('\\', "/").to_ascii_lowercase();
        if !catalog.has_overlays() {
            if let Some(metadata) = self.metadata.get(&key).and_then(Weak::upgrade) {
                return Ok(metadata);
            }
        }
        self.metadata.retain(|_, value| value.strong_count() != 0);
        let mut sources = SourceRead {
            catalog,
            cancelled,
            texts: HashMap::new(),
        };
        let mut class_name = None;
        let mut entity_model = None;
        let chain = entities::player::entity_chain(&key, &mut |path| sources.read(path))?;
        let ai_tampering_declared = entities::player::entity_component_from_chain(
            &chain, "cpntaitampering", |path| sources.read(path))?.is_some();
        for layer in chain {
            if let Some(class) = entities::scene_motion::atom(&layer, "class") {
                class_name = Some(class.to_owned());
            }
            let render = entities::scene_motion::child(
                entities::scene_motion::child(&layer, "edit"),
                "renderModelInfo",
            );
            if let Some(model) = entities::scene_motion::atom(render, "model") {
                entity_model = (!model.is_empty() && model != "NULL").then(|| model.to_owned());
            }
        }
        let class_name =
            class_name.ok_or_else(|| format!("{entity}: equipment entity has no class"))?;
        let mut components =
            entities::player::entity_components_with_prefix(&key, "cpntweapon", |path| {
                sources.read(path)
            })?;
        components.extend(entities::player::entity_components_with_prefix(
            &key,
            "cpntpower",
            |path| sources.read(path),
        )?);
        for kind in ["cpntheart", "cpnttimeshiftdevice", "cpntitemspyglass", "cpntarmor"] {
            if let Some(fields) =
                entities::player::entity_component(&key, kind, |path| sources.read(path))?
            {
                components.push((kind.to_owned(), fields));
            }
        }
        let [(kind, fields)] = components.as_slice() else {
            return Err(format!(
                "{entity}: expected one concrete player weapon equipment component"
            ));
        };
        // Exact component-class constructor defaults, not inventory type,
        // model name, file name or an inferred hand/weapon category.
        let (archetype, slot, is_ranged) = match kind.as_str() {
            "cpntweaponmeleeplayer" => (1, 0, false), // 633c70
            "cpntweapongun" | "cpntweaponcrossbow" => (2, 1, true), // 44dae0/62b1b0 -> 6376c0
            "cpntweaponplaceable" => (3, 1, false),   //450ad0 ->6376c0
            "cpntweaponthrowable" => (4, 1, false),   //632bf0 ->6376c0
            "cpntheart" => (7, 1, false),             //611800 ->619ea0
            "cpnttimeshiftdevice" => (9, 1, false),   //44bbe0 ->619ea0
            "cpntitemspyglass" => (6, 4, false),      //61b970 ->619ea0
            "cpntarmor" => (8, 4, false),             // arkDeclCpntArmor ->619ea0
            // Each concrete power constructor calls61ba50 ->619ea0(5,1).
            "cpntpowerbendtime"
            | "cpntpowerblink"
            | "cpntpowershadowwalk"
            | "cpntpowermesmerize"
            | "cpntpowerdoppelganger"
            | "cpntpowergrapple"
            | "cpntpowerpossession"
            | "cpntpowerdarkvision"
            | "cpntpowerwindblast"
            | "cpntpowerdevouringswarm"
            | "cpntpowerphantasmallink" => (5, 1, false),
            _ => {
                return Err(format!(
                    "{entity}: equipment constructor {kind} is not implemented"
                ));
            }
        };
        let equipment = Equipment::read(fields, archetype, slot, &mut enumeration)?;
        let melee = (kind == "cpntweaponmeleeplayer")
            .then(|| crate::gameplay::player::melee::Declaration::acquire(catalog, fields)).transpose()?;
        let ranged = is_ranged
            .then(|| {
                Ranged::read(
                    fields,
                    equipment.clone(),
                    if kind == "cpntweapongun" {
                        RangedFamily::Gun
                    } else {
                        RangedFamily::Crossbow
                    },
                )
            })
            .transpose()?;

        check_active(cancelled)?;
        let sound_settings =
            entities::player::entity_component(&key, "cpntaudio", |path| sources.read(path))?
                .map(|fields| crate::gameplay::sound::component::Settings::read(&fields))
                .transpose()?;
        let heart = (kind == "cpntheart").then(||
            crate::gameplay::player::weapons::heart::Definition::read(fields).map(Arc::new))
            .transpose()?;
        let heart_barks = if heart.is_some() {
            entities::player::entity_component(&key, "cpntaibarksdevice", |path| sources.read(path))?
                .map(|fields| {
                    let speech = entities::player::entity_component(&key, "cpntspeech", |path| sources.read(path))?
                        .ok_or_else(|| format!("{key}: AIBarks_Device requires its actual Speech component"))?;
                    crate::gameplay::player::weapons::heart::barks::Definition::read(catalog, &fields, &speech).map(Arc::new)
                }).transpose()?
        } else { None };
        let throwable = (kind == "cpntweaponthrowable").then(||
            crate::gameplay::player::weapons::throwable::Definition::read(fields).map(Arc::new))
            .transpose()?;
        let grenade_fields = entities::player::entity_component(&key, "cpntgrenade", |path| sources.read(path))?;
        let grenade = grenade_fields.as_ref()
            .map(|fields| crate::assets::grenades::Definition::read(fields).map(Arc::new)).transpose()?;
        let grenade_scheduling = grenade_fields.as_ref().map(|fields| {
            entities::components::scheduling(fields,
                crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0)).map(Arc::new)
        }).transpose()?;
        let metadata = Arc::new(Metadata {
            entity: key.clone(),
            class_name,
            ai_tampering_declared,
            model: entity_model,
            sound_settings,
            component_kind: kind.clone(),
            equipment,
            melee,
            ranged,
            heart,
            heart_barks,
            throwable,
            grenade,
            grenade_scheduling,
        });
        if !catalog.has_overlays() {
            self.metadata.insert(key, Arc::downgrade(&metadata));
        }
        Ok(metadata)
    }

    pub fn acquire(
        &mut self,
        catalog: &Catalog,
        entity: &str,
        mut enumeration: impl FnMut(&str, &str) -> Option<i32>,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> Result<Arc<Descriptor>, String> {
        check_active(cancelled)?;
        if self.catalog.is_some_and(|id| id != catalog.identity()) {
            return Err("equipment sources belong to another level catalog".into());
        }
        self.catalog = Some(catalog.identity());
        let key = entity.replace('\\', "/").to_ascii_lowercase();
        if !catalog.has_overlays() {
            if let Some(value) = self.entities.get(&key).and_then(Weak::upgrade) {
                return Ok(value);
            }
        }
        self.entities.retain(|_, value| value.strong_count() != 0);
        // Temporary handoff only: the component resolver walks the same
        // entity chain for each requested class, but each source reads once.
        let mut sources = SourceRead {
            catalog,
            cancelled,
            texts: HashMap::new(),
        };
        let metadata = self.metadata(catalog, &key, &mut enumeration, cancelled)?;
        let mut model = None;
        for fields in entities::player::entity_chain(&key, &mut |path| sources.read(path))? {
            let edit = entities::scene_motion::child(&fields, "edit");
            let info = entities::scene_motion::child(edit, "renderModelInfo");
            if let Some(value) = entities::scene_motion::atom(info, "model") {
                model = Some(value.to_owned());
            }
        }
        let sound_settings = metadata.sound_settings;
        let Some(model) = model else {
            check_active(cancelled)?;
            let descriptor = Arc::new(Descriptor { metadata, visual: None, sound_settings });
            if !catalog.has_overlays() {
                self.entities.insert(key, Arc::downgrade(&descriptor));
            }
            return Ok(descriptor);
        };
        let anim =
            entities::player::entity_component(&key, "cpntanimevolved", |path| sources.read(path))?
                .ok_or_else(|| format!("{entity}: equipment has no animation component"))?;
        let animation_tree = entities::scene_motion::atom(&anim, "m_animTree")
            .filter(|name| !name.is_empty() && *name != "NULL")
            .ok_or_else(|| format!("{entity}: animation component has no tree"))?
            .to_owned();
        let tree_path = format!("generated/decls/animtree/{animation_tree}.decl");
        let tree_source = sources
            .read(&tree_path)?
            .ok_or_else(|| format!("equipment tree missing: {tree_path}"))?;
        let default_input = metadata
            .equipment
            .default_input
            .as_deref()
            .ok_or_else(|| format!("{entity}: equipment has no default animation input"))?;
        tree::midnight_default(&tree_source, default_input)
            .map_err(|error| format!("{tree_path}: {error}"))?;
        let default_alias = metadata
            .equipment
            .default_alias
            .as_deref()
            .ok_or_else(|| format!("{entity}: equipment has no default animation alias"))?;
        check_active(cancelled)?;
        let rig = Rig::acquire(catalog, &model)?;
        check_active(cancelled)?;
        let (default_clip, projectile_offset, animations, animation_aliases) = {
            let mut clips = Clips {
                catalog,
                rig: &rig,
                rest: &rig.skeleton.rest,
                animations: Vec::new(),
                loaded: HashMap::new(),
            };
            let default_clip = if rig.alias(default_alias).is_some() {
                Some(clips.clip_cancellable(default_alias, cancelled)?)
            } else {
                None
            };
            let projectile_offset = if let Some(settings) = metadata.ranged.as_ref().filter(|settings| settings.family == RangedFamily::Crossbow) {
                if let Some(alias) = settings.fired_animation.as_deref() { clips.clip_cancellable(alias, cancelled)?; }
                Some(firing::crossbow_offset(&mut sources, settings, &rig, &mut clips, cancelled)?)
            } else { None };
            (default_clip, projectile_offset, clips.animations, clips.loaded)
        };
        check_active(cancelled)?;
        let stem = model
            .strip_suffix(".md6")
            .ok_or_else(|| format!("equipment model is not MD6: {model}"))?;
        let resource = format!("generated/basemodel/{stem}.bmd6model");
        let geometry = bmd6::ModelSources::new(catalog).acquire(catalog, &resource)?;
        check_active(cancelled)?;
        let descriptor = Arc::new(Descriptor {
            metadata,
            visual: Some(Visual { model, rig, geometry, animation_tree, animations, animation_aliases, default_clip, projectile_offset }),
            sound_settings,
        });
        if !catalog.has_overlays() {
            self.entities.insert(key, Arc::downgrade(&descriptor));
        }
        Ok(descriptor)
    }
}

/// Enum constants registered by1419fb190/1419fb470. Callers use this same
/// table for numeric/declaration metadata instead of inferring a hand/type.
pub fn enumeration(kind: &str, value: &str) -> Option<i32> {
    //141a1b2e0's explicit -1 sentinel precedes the eight Flash types.
    if kind == "arkUIElementUICrosshairType_t" {
        const TYPES: [&str; 9] = [
            "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_NULL", "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_POWER",
            "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_GUN", "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_CROSSBOW",
            "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_MINE", "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_GRENADE",
            "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_STICKY_GRENADE", "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_GRENADE_LOB",
            "ARK_HUD_ELEMENT_CROSSHAIR_TYPE_STICKY_GRENADE_LOB",
        ];
        return value.parse().ok().or_else(|| TYPES.iter().position(|name| *name == value).map(|index| index as i32 - 1));
    }
    if kind == "arkPlayerStateId_t" {
        // The existing complete registration table141a13470 is shared with
        // interaction rules; equipment quick-use states keep the same enum.
        return value.parse().ok().or_else(|| {
            crate::gameplay::interaction_enums::PLAYER_STATES
                .iter()
                .find_map(|(name, id)| (*name == value).then_some(*id))
        });
    }
    let table: &[&str] = match kind {
        "arkEquipmentSlot_t" => &[
            "SLOT_RIGHT_HAND",
            "SLOT_LEFT_HAND",
            "SLOT_RIGHT_HAND2",
            "SLOT_LEFT_HAND2",
            "SLOT_HEAD",
            "SLOT_HEAD2",
            "SLOT_RIGHT_LEG",
            "SLOT_LEFT_LEG",
            "SLOT_RIGHT_LEG2",
            "SLOT_LEFT_LEG2",
            "SLOT_RIGHT_ARM",
            "SLOT_LEFT_ARM",
            "SLOT_RIGHT_ARM2",
            "SLOT_LEFT_ARM2",
            "SLOT_TORSO_FRONT",
            "SLOT_TORSO_BACK",
            "SLOT_MAX",
        ],
        "arkEquipmentArchetype_t" => &[
            "ARCHETYPE_NONE",
            "ARCHETYPE_WEAPON_MELEE",
            "ARCHETYPE_WEAPON_RANGED",
            "ARCHETYPE_WEAPON_PLACEABLE",
            "ARCHETYPE_WEAPON_THROWABLE",
            "ARCHETYPE_POWER",
            "ARCHETYPE_SPYGLASS",
            "ARCHETYPE_HEART",
            "ARCHETYPE_ARMOR",
            "ARCHETYPE_TIMESHIFTDEVICE",
            "ARCHETYPE_MISSION",
            "ARCHETYPE_MAX",
        ],
        _ => return None,
    };
    value.parse().ok().or_else(|| {
        table
            .iter()
            .position(|name| *name == value)
            .map(|index| index as i32)
    })
}

fn check_active(cancelled: Option<&dyn Fn() -> bool>) -> Result<(), String> {
    if cancelled.is_some_and(|cancelled| cancelled()) {
        Err("equipment acquisition cancelled".into())
    } else {
        Ok(())
    }
}

struct SourceRead<'a> {
    catalog: &'a Catalog,
    cancelled: Option<&'a dyn Fn() -> bool>,
    texts: HashMap<String, Option<String>>,
}
impl SourceRead<'_> {
    fn read(&mut self, path: &str) -> Result<Option<String>, String> {
        check_active(self.cancelled)?;
        let mut key = path.replace('\\', "/").to_ascii_lowercase();
        //Component source suffixes and serialized declaration-manager names
        //differ (for example AIBarks_Device). Resolve through the same catalog
        //type metadata as the runtime factory, never a directory alias.
        if let Some((kind, source)) = key.strip_prefix("generated/decls/").and_then(|value| value.split_once('/')) {
            if kind.starts_with("cpnt") {
                let suffix = source.strip_suffix(".decl").and_then(|value| value.rsplit_once('.')).map(|(_, suffix)| suffix)
                    .ok_or_else(|| format!("equipment component source has no declaration suffix: {key}"))?;
                let actual = self.catalog.component_declaration_kind(suffix)?;
                key = format!("generated/decls/{}/{source}", actual.to_ascii_lowercase());
            }
        }
        if let Some(text) = self.texts.get(&key) {
            return Ok(text.clone());
        }
        let text = self
            .catalog
            .read_cancellable(&key, self.cancelled)
            .map_err(|error| error.to_string())?
            .map(entities::owned_declaration_text);
        self.texts.insert(key, text.clone());
        Ok(text)
    }
}
