//! Host storage for the ordinary decal-FX family. The installed declaration
//! owner retains material names; GPU material assets remain renderer-owned.
use crate::{assets::decal::DecalMaterials, rendering::decals::CommittedDecal};
use std::sync::Arc;

pub(super) type DecalReferences = Arc<DecalMaterials<String>>;
pub(super) type DecalSystems = crate::rendering::decals::systems::DecalSystems<
    DecalReferences,
    CommittedDecal<DecalReferences>,
>;

pub(super) fn new_systems() -> DecalSystems {
    let limit = crate::platform::console::shared()
        .read()
        .cvars
        .int("r_maxDynamicDecals")
        .expect("registered decal budget");
    DecalSystems::new(limit as usize).expect("registered decal budget range")
}
