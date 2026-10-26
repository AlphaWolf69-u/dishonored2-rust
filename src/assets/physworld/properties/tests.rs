use super::*;
#[test]
#[ignore = "requires installed level physics archives"]
fn installed_body_shape_property_arrays() {
    let catalog =
        crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let paths: std::collections::BTreeSet<_> = catalog
        .paths_with_suffix(".bwm")
        .into_iter()
        .filter_map(|path| {
            crate::assets::level::LevelResources::resolve(&catalog, &path)
                .unwrap()
                .physics
        })
        .collect();
    let mut counts = [0usize; 4];
    for path in paths {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let world = super::super::parse(&bytes).unwrap();
        for body in world.body_infos(&bytes).unwrap() {
            let properties = world
                .shape_properties(&bytes, body.shape)
                .unwrap_or_else(|e| panic!("{path} shape{}: {e}", body.shape));
            for property in &properties {
                assert!(
                    property.object < world.items.len(),
                    "{path} property reference{}",
                    property.object
                );
            }
            counts[2] += properties.len();
            if let Some(frame) = world.shape_mass_properties(&bytes, body.shape).unwrap() {
                assert!(frame.mass.is_finite() && frame.volume.is_finite(), "{path}");
                counts[3] += 1;
            }
            counts[1] += 1;
        }
        counts[0] += 1;
    }
    assert_eq!(&counts[..2], &[36, 530]);
    println!("Installed worlds/bodies/properties/cached-mass={counts:?}");
}

#[test]
fn property_lookup_matches_executed_reverse_search() {
    for (case, line) in include_str!("../../../../tests/fixtures/shape_properties.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let key = words.next().unwrap() as u16;
        let properties: Vec<_> = (0..4)
            .map(|_| ShapeProperty {
                object: words.next().unwrap() as usize,
                key: words.next().unwrap() as u16,
                flags: 0,
            })
            .collect();
        assert_eq!(
            property_object(&properties, key).unwrap_or(0),
            words.next().unwrap() as usize,
            "case {case}"
        );
        assert!(words.next().is_none());
    }
}
