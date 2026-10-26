use super::*;
#[test]
#[ignore = "requires installed physics-material declarations"]
fn installed_physics_material_settings_match_executed_conversion() {
    let catalog =
        crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut settings = HashMap::new();
    let mut flag_counts = std::collections::BTreeMap::<u16, usize>::new();
    for path in catalog.paths_with_suffix(".decl") {
        let Some(name) = path
            .strip_prefix("generated/decls/physicsmaterial/")
            .and_then(|s| s.strip_suffix(".decl"))
        else {
            continue;
        };
        let material = collision_settings(name, |path| {
            catalog
                .read(path)
                .map(|b| b.map(|b| super::super::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        })
        .unwrap_or_else(|e| panic!("{path}: {e}"));
        *flag_counts.entry(material.flags).or_default() += 1;
        settings.insert(name.to_owned(), material);
    }
    assert_eq!(settings.len(), 314);
    for line in include_str!("../../../../tests/fixtures/physics_material_collision.txt").lines() {
        let mut row = line.split_whitespace();
        let name = row.next().unwrap();
        let expected: Vec<u32> = row.map(|s| u32::from_str_radix(s, 16).unwrap()).collect();
        let value = settings[name];
        assert_eq!(
            [
                u32::from(value.flags),
                value.version as u32,
                value.blocker,
                u32::from(value.legacy_flags)
            ],
            expected[..],
            "{name}"
        );
    }
    println!(
        "Installed physics material settings={} collision flags={flag_counts:?}",
        settings.len()
    );
}
