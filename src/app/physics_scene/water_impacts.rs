//!651020 material pointers retained at accepted first submersion.
//!08b6d0 supplies the registered default declaration for a NULL water slot.
use crate::{assets::entities::physics_material::resources::Definition,
    gameplay::materials::MaterialOwner};
use std::sync::Arc;

pub(super) fn material_leases(
    materials: &MaterialOwner<usize, usize>,
    source: u16,
    target: Option<usize>,
) -> Result<(Arc<Definition>, Arc<Definition>), String> {
    let source = materials.definition_for_id(source)?.clone();
    let target = match target {
        Some(handle) => materials.contact_definitions().get(handle)
            .ok_or("water impact declaration lease absent")?.clone(),
        //0932d0 binds the same08b6d0 code-resource declaration to preset0.
        None => materials.definition_for_id(0)?.clone(),
    };
    Ok((source, target))
}
