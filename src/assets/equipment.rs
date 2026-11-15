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
    equipment::{EquipmentBinding, EquipmentInput, EquipmentTree},
    weapons::{Equipment, Ranged},
};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

mod tree;

/// Immutable selected-entity projection. No live playback state or independent
/// material/geometry cache lives here. The host retains this lease while the
/// inventory entity exists and through its renderer's acknowledgement.
pub struct Descriptor {
    pub entity: String,
    pub component_kind: String,
    pub equipment: Equipment,
    pub ranged: Option<Ranged>,
    pub model: String,
    pub rig: Arc<Rig>,
    pub geometry: Arc<bmd6::ModelInfo>,
    pub animation_tree: String,
    pub animations: Vec<crate::edge_animation::EdgeAnimation>,
    pub animation_aliases: HashMap<String, usize>,
    default_clip: usize,
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
        let (joint, translation, rotation) = owner.tag(tag)?;
        Ok((binding, joint, (translation, rotation)))
    }

    pub fn instantiate(
        &self,
        body: &Rig,
        arms: &[Arc<Rig>; 2],
        equipped: bool,
    ) -> Result<EquipmentTree, String> {
        let (binding, bind_joint, bind) = self.binding(body, arms, equipped)?;
        Ok(EquipmentTree {
            rig: self.rig.clone(),
            model: self.model.clone(),
            animations: self.animations.clone(),
            animation_aliases: self.animation_aliases.clone(),
            input: EquipmentInput::DefaultAnimation {
                clip: self.default_clip,
                time: 0.0,
            },
            binding,
            bind_joint,
            bind,
            visible: equipped || !self.equipment.hide_unequipped,
        })
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
}

impl Sources {
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
        let components =
            entities::player::entity_components_with_prefix(&key, "cpntweapon", |path| {
                sources.read(path)
            })?;
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
            _ => {
                return Err(format!(
                    "{entity}: equipment constructor {kind} is not implemented"
                ));
            }
        };
        let equipment = Equipment::read(fields, archetype, slot, &mut enumeration)?;
        let ranged = is_ranged
            .then(|| Ranged::read(fields, equipment.clone()))
            .transpose()?;
        let mut model = None;
        for fields in entities::player::entity_chain(&key, &mut |path| sources.read(path))? {
            let edit = entities::scene_motion::child(&fields, "edit");
            let info = entities::scene_motion::child(edit, "renderModelInfo");
            if let Some(value) = entities::scene_motion::atom(info, "model") {
                model = Some(value.to_owned());
            }
        }
        let model = model.ok_or_else(|| format!("{entity}: equipment has no render model"))?;
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
        let default_input = equipment
            .default_input
            .as_deref()
            .ok_or_else(|| format!("{entity}: equipment has no default animation input"))?;
        tree::midnight_default(&tree_source, default_input)
            .map_err(|error| format!("{tree_path}: {error}"))?;
        let default_alias = equipment
            .default_alias
            .as_deref()
            .ok_or_else(|| format!("{entity}: equipment has no default animation alias"))?;
        check_active(cancelled)?;
        let rig = Rig::acquire(catalog, &model)?;
        check_active(cancelled)?;
        let (default_clip, animations, animation_aliases) = {
            let mut clips = Clips {
                catalog,
                rig: &rig,
                rest: &rig.skeleton.rest,
                animations: Vec::new(),
                loaded: HashMap::new(),
            };
            let default_clip = clips.clip(default_alias)?;
            (default_clip, clips.animations, clips.loaded)
        };
        check_active(cancelled)?;
        let stem = model
            .strip_suffix(".md6")
            .ok_or_else(|| format!("equipment model is not MD6: {model}"))?;
        let resource = format!("generated/basemodel/{stem}.bmd6model");
        let geometry = bmd6::ModelSources::new(catalog).acquire(catalog, &resource)?;
        check_active(cancelled)?;
        let descriptor = Arc::new(Descriptor {
            entity: key.clone(),
            component_kind: kind.clone(),
            equipment,
            ranged,
            model,
            rig,
            geometry,
            animation_tree,
            animations,
            animation_aliases,
            default_clip,
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
        let key = path.replace('\\', "/").to_ascii_lowercase();
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
