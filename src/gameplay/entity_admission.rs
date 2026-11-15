//! Map entity admission by the level spawn owner (8adb40 / 0147d0).
//! The caller supplies the actual decoded declaration flags and layer state.

/// 0147d0 compares registered class identities exactly, not by inheritance.
pub fn spawnable_class(class: &str, declaration_flags: u8, debug_mode: i32) -> bool {
    if declaration_flags & 0x20 != 0 { return false; }
    if debug_mode >= 2 && class == "idInfoDebugText" { return false; }
    !matches!(class,
        "idStaticEntity" | "arkMassiveLOD" | "idReferenceMap" |
        "arkEnvironmentProbe" | "arkPrefabEntity" | "arkDecalEntity" | "idSplinePath")
}

/// The normal initial map command passes `initial=true, all_layers=false`.
/// Dynamic layer requests must also exclude entities already in active layers.
pub fn selected_layers(
    layers: &[String], requested: &[String], active: &[String],
    initial: bool, all_layers: bool,
) -> bool {
    if all_layers { return true; }
    if layers.is_empty() { return initial; }
    let requested_match = layers.iter().any(|layer| requested.iter().any(|item| item == layer));
    let already_active = !initial && layers.iter().any(|layer| active.iter().any(|item| item == layer));
    requested_match && !already_active
}
