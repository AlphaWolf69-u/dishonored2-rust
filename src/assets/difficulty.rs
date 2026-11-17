//! Typed difficulty declaration identities and immutable source tables.
//! Registrations141387290/1413872f0, constructors6472a0/647550;
//! common registry phases0607f0/05feb0/0661e0/060b60/060180.
use crate::archive::Catalog;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Preset {
    pub name: u32,
    pub values: [i32; 24],
}
#[derive(Clone, Debug, Default)]
pub struct Slot {
    pub label: u32,
    pub category: u32,
    pub description: u32,
    pub values_description: u32,
    pub levels: [u32; 3],
    pub availability: i32,
    pub priority: i32,
}
#[derive(Debug)]
pub struct Definition {
    pub presets: Arc<[Preset]>,
    pub custom_name: u32,
    pub stepper: Slot,
    pub slots: Arc<[Slot]>,
    pub limited_lean_factors: [f32; 2],
    pub incoming_damage_factors: [f32; 3],
    pub incoming_damage_back_factors: [f32; 3],
    pub health_heal_factors: [f32; 3],
    pub health_potion_factors: [f32; 3],
    pub mana_potion_factors: [f32; 3],
    pub mana_heal_factors: [f32; 3],
    pub health_regen_amount_scale: f32,
    ///647ee0: declaration+174, indexed by the active ammo-quantity level.
    pub ammo_quantity_offsets: [i32; 3],
    // Host: keep declaration generation leases while direct contiguous
    // runtime preset tables avoid an extra pointer lookup per value read.
    pub(crate) preset_leases: Vec<Arc<Preset>>,
}
const MANAGER: &str =
    "generated/decls/difficultymanager/difficulty/difficultysettings.difficultymanager.decl";
fn shared(catalog: &Catalog) -> bool {
    !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_DIFFICULTY_RESOURCES").as_deref() != Ok("1")
}
pub(crate) fn acquire_manager(
    catalog: &Catalog,
    parse: impl FnOnce(u8) -> Result<Definition, String>,
) -> Result<Arc<Definition>, String> {
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let mut decoded = false;
    let load = |marker| {
        decoded = true;
        let _tag = crate::platform::allocation_tags::tag("Difficulty declarations");
        parse(marker).map(Arc::new)
    };
    let value = catalog
        .difficulty_definitions
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "difficulty manager registry is poisoned")?
        .acquire_with_sharing(MANAGER, 0, shared(catalog), load);
    if trace {
        eprintln!(
            "DIFFICULTY_MANAGER_ACQUIRE decoded={decoded} success={}",
            value.is_ok()
        );
    }
    value
}
pub(crate) fn acquire_preset(
    catalog: &Catalog,
    name: &str,
    marker: u8,
    parse: impl FnOnce() -> Result<Preset, String>,
) -> Result<Arc<Preset>, String> {
    let path = format!("generated/decls/difficultypreset/{name}.decl")
        .replace('\\', "/")
        .to_ascii_lowercase();
    let mut decoded = false;
    let load = |_| {
        decoded = true;
        let _tag = crate::platform::allocation_tags::tag("Difficulty declarations");
        parse().map(Arc::new)
    };
    let value = catalog
        .difficulty_presets
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "difficulty preset registry is poisoned")?
        .acquire_with_sharing(&path, marker, shared(catalog), load);
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!(
            "DIFFICULTY_PRESET_ACQUIRE path={path} decoded={decoded} success={}",
            value.is_ok()
        );
    }
    value
}
pub(crate) fn register_preload(
    catalog: &Catalog,
    kind: &str,
    path: &str,
    marker: u8,
) -> Result<(), String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    if kind.eq_ignore_ascii_case("difficultyManager") {
        catalog
            .difficulty_definitions
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "difficulty manager registry is poisoned")?
            .register(&path, marker, false);
    } else {
        catalog
            .difficulty_presets
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "difficulty preset registry is poisoned")?
            .register(&path, marker, false);
    }
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.difficulty_definitions.get() {
        owner
            .lock()
            .map_err(|_| "difficulty manager registry is poisoned")?
            .promote_bootstrap();
    }
    if let Some(owner) = catalog.difficulty_presets.get() {
        owner
            .lock()
            .map_err(|_| "difficulty preset registry is poisoned")?
            .promote_bootstrap();
    }
    Ok(())
}
fn slice_storage<T>(slice: &[T]) -> usize {
    std::alloc::Layout::new::<[usize; 2]>()
        .extend(std::alloc::Layout::for_value(slice))
        .expect("difficulty slice layout")
        .0
        .pad_to_align()
        .size()
}
fn nested(value: &Definition) -> usize {
    slice_storage(&value.presets)
        + slice_storage(&value.slots)
        + value.preset_leases.capacity() * std::mem::size_of::<Arc<Preset>>()
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.difficulty_definitions.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "difficultyManager", marker);
        owner.report(
            catalog.identity(),
            "difficultyManager",
            "scope-released",
            nested,
        );
    }
    if let Some(owner) = catalog.difficulty_presets.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "difficultyPreset", marker);
        owner.report(
            catalog.identity(),
            "difficultyPreset",
            "scope-released",
            |_| 0,
        );
    }
}

pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.difficulty_definitions.get() {
        owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .report(
                catalog.identity(),
                "difficultyManager",
                "catalog-retiring",
                nested,
            );
    }
    if let Some(owner) = catalog.difficulty_presets.get() {
        owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .report(
                catalog.identity(),
                "difficultyPreset",
                "catalog-retiring",
                |_| 0,
            );
    }
}
