//! Pickable_Simple hold-and-throw projection (14076e0e0/141738440).
//! The level owns these compact declarations; no inventory item, model, rigid
//! body or converted asset is acquired by this reader.
use super::{Definition, Document, Value, components, field, scope};
use crate::gameplay::interaction_definition::InteractionDef;
use std::{collections::HashMap, sync::Arc};

type Fields = [(String, Value)];

use crate::gameplay::pickable_component::{HoldProperties, PickableDefinition};

#[derive(Clone)]
pub(crate) struct Binding {
    pub entity: String,
    pub definition: Arc<PickableDefinition>,
}

fn child<'a>(fields: &'a Fields, name: &str) -> &'a Fields {
    scope(field(fields, name)).unwrap_or(&[])
}
fn atom<'a>(fields: &'a Fields, name: &str) -> Option<&'a str> {
    components::atom(fields, name)
}
fn flag(fields: &Fields, name: &str, default: bool) -> Result<bool, String> {
    match atom(fields, name) {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(value) => Err(format!("invalid holdable {name}: {value}")),
    }
}
fn number(fields: &Fields, name: &str, default: f32) -> Result<f32, String> {
    atom(fields, name).map_or(Ok(default), |value| {
        value
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("invalid holdable {name}: {value}"))
    })
}
fn vector(fields: &Fields, name: &str, default: [f32; 3]) -> Result<[f32; 3], String> {
    let fields = child(fields, name);
    Ok([
        number(fields, "x", default[0])?,
        number(fields, "y", default[1])?,
        number(fields, "z", default[2])?,
    ])
}

/// Share inherited/map component projections, preserving inline identities.
/// 76e0e0 constructs isHold=false; eligibility is the authored flag, never a
/// bottle/model-name heuristic or the presence of a Throwable component.
pub(crate) fn bindings(
    catalog: &crate::archive::Catalog,
    document: &Document,
) -> Result<Vec<Binding>, String> {
    let declarations: HashMap<&str, &Definition> = document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "component")
        .map(|definition| (definition.id.as_str(), definition))
        .collect();
    let mut read = |path: &str| {
        catalog
            .read(path)
            .map_err(|error| error.to_string())
            .map(|bytes| bytes.map(super::owned_declaration_text))
    };
    let mut sources = HashMap::new();
    let mut definitions: HashMap<String, Arc<PickableDefinition>> = HashMap::new();
    let mut result = Vec::new();
    for entity in document
        .definitions
        .iter()
        .filter(|definition| definition.kind == "entity")
    {
        for (slot, value) in child(child(&entity.fields, "edit"), "m_componentDecls") {
            if !slot.starts_with("item_add[") {
                continue;
            }
            let Some(entry) = scope(Some(value)) else {
                continue;
            };
            let derived = match components::entry_class(entry, &declarations)?.as_deref() {
                Some("cpntpickable_bonecharm" | "cpntpickablebonecharm") => true,
                Some("cpntpickable_simple" | "cpntpickablesimple") => false,
                _ => continue,
            };
            let Some(fields) = components::entry_fields(
                entry,
                if derived { "cpntpickable_bonecharm" } else { "cpntpickable_simple" },
                &declarations,
                &mut sources,
                &mut read,
            )?
            else {
                continue;
            };
            let props = child(&fields, "m_holdAndThrowProps");
            let interaction = child(&fields, "m_pickupInteraction");
            if !flag(props, "m_isHoldAndThrowObject", false)? {
                let item = atom(&fields, "m_inventoryItem")
                    .filter(|item| !item.is_empty() && !item.eq_ignore_ascii_case("NULL"));
                // The read frontend already prepares its Pickable component.
                // All other inventory pickups need the same component as a
                // holdable to enter LootObject and acknowledge the collection.
                if item.is_none() || atom(interaction, "m_playerState") == Some("PLAYER_STATE_READ_NOTE") {
                    continue;
                }
            }
            let identity = atom(entry, "mapComponentDecl")
                .map(|path| format!("map:{path}"))
                .or_else(|| {
                    field(entry, "inlineDecl")
                        .is_none()
                        .then(|| {
                            atom(entry, "inheritedDecl").map(|path| {
                                format!(
                                    "inherited:{}",
                                    path.replace('\\', "/").to_ascii_lowercase()
                                )
                            })
                        })
                        .flatten()
                });
            let definition =
                if let Some(shared) = identity.as_ref().and_then(|key| definitions.get(key)) {
                    shared.clone()
                } else {
                    let definition = if derived { charm_definition(catalog, &fields)? } else { definition(catalog, &fields)? };
                    if !catalog.has_overlays() {
                        if let Some(key) = identity {
                            definitions.insert(key, definition.clone());
                        }
                    }
                    definition
                };
            result.push(Binding { entity: entity.id.clone(), definition });
        }
    }
    Ok(result)
}

/// The canonical PickableDefinition reader and readable reader share this
/// projection, rather than allocating a separate component for held objects.
pub(crate) fn hold_properties(fields: &Fields) -> Result<Option<HoldProperties>, String> {
    let props = child(fields, "m_holdAndThrowProps");
    if !flag(props, "m_isHoldAndThrowObject", false)? {
        return Ok(None);
    }
    let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let matrix = child(child(props, "m_angleOffset"), "mat");
    let mut angle_offset = identity;
    for index in 0..3 {
        angle_offset[index] = vector(matrix, &format!("mat[{index}]"), identity[index])?;
    }
    let encumbrance = match atom(props, "m_encumbrance") {
        None | Some("ARK_ENCUMBRANCE_SMALL" | "0") => 0,
        Some("ARK_ENCUMBRANCE_LARGE" | "1") => 1,
        Some(value) => return Err(format!("invalid holdable encumbrance: {value}")),
    };
    Ok(Some(HoldProperties {
        center_offset: vector(props, "m_centerOffset", [0.; 3])?,
        angle_offset,
        encumbrance,
        both_hands: flag(props, "m_needBothHands", true)?,
        //76e0e0 copies DAT14398e1f0; startup14138b300 interns
        //the installed "camera_tag" string at141c5a518.
        attach_tag: match atom(props,"m_attachTag"){
            None=>"camera_tag",Some(tag) if tag.eq_ignore_ascii_case("NULL")=>"",Some(tag)=>tag,
        }.to_owned(),
        collide_with_sword: flag(props, "m_collideWithSword", false)?,
        throw_factor: number(props, "m_throwFactor", 1.)?,
    }))
}

/// Canonical concrete declaration projection for both map components and
/// actual runtime entity-definition births. The caller owns declaration cache
/// identity; no extra resource cache or actor is created here.
pub(crate) fn definition(
    catalog: &crate::archive::Catalog,
    fields: &Fields,
) -> Result<Arc<PickableDefinition>, String> {
    pickup_definition(catalog, fields, None)
}

pub(crate) fn charm_definition(catalog: &crate::archive::Catalog, fields: &Fields) -> Result<Arc<PickableDefinition>, String> {
    pickup_definition(catalog, fields, Some(crate::assets::bone_charms::Placement::read(fields)?))
}

fn pickup_definition(catalog: &crate::archive::Catalog, fields: &Fields,
    charm: Option<crate::assets::bone_charms::Placement>) -> Result<Arc<PickableDefinition>, String> {
    let mut interaction = InteractionDef::from_fields(child(fields, "m_pickupInteraction"))?;
    if field(child(fields, "m_pickupInteraction"), "m_playerState").is_none() {
        interaction.player_state = 47;
    }
    let mut read = |path: &str| {
        catalog
            .read(path)
            .map_err(|error| error.to_string())
            .map(|bytes| bytes.map(super::owned_declaration_text))
    };
    let begin_pickup_sound = atom(fields, "m_beginPickupSoundEvent")
        .filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL"))
        .map(|path| crate::assets::wwise::event_resources::acquire_with(catalog, path, &mut read))
        .transpose()?;
    Ok(Arc::new(PickableDefinition {
        charm,
        hold: hold_properties(fields)?,
        item: atom(fields, "m_inventoryItem")
            .filter(|path| !path.eq_ignore_ascii_case("NULL"))
            .unwrap_or("")
            .to_owned(),
        quantity: atom(fields, "m_quantity")
            .unwrap_or("0")
            .parse()
            .map_err(|_| "invalid pickable quantity")?,
        begin_pickup_sound,
        shadow_grapple: flag(fields, "m_canBeShadowGrappled", true)?,
        usable_by_player: flag(fields, "m_usableByPlayer", true)?,
        invisible_npc_loot: flag(fields, "m_invisibleNpcLoot", false)?,
        pickup_interaction: interaction,
        interaction_labels: [atom(child(fields,"m_pickupInteraction"),"m_name").unwrap_or("").to_owned(),
            atom(child(fields,"m_pickupInteraction"),"m_usageText").unwrap_or("").to_owned()],
        interaction_state: atom(child(fields,"m_pickupInteraction"),"m_playerState").unwrap_or("").to_owned(),
        scheduling: components::scheduling(
            fields,
            crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(
                0,
            ),
        )?,
    }))
}
