use crate::{archive::Catalog, assets::level::LevelResources};
use std::collections::{BTreeMap, BTreeSet};

#[test]
#[ignore = "requires installed environment-trigger HKX assets and entities"]
fn installed_environment_trigger_polytopes_keep_query_data() {
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut paths = BTreeSet::new();
    for world in catalog.paths_with_suffix(".bwm") {
        let level = LevelResources::resolve(&catalog, &world).unwrap();
        let bytes = catalog.read(&level.entities).unwrap().unwrap();
        let document = crate::entities::parse(&crate::entities::declaration_text(&bytes)).unwrap();
        for volume in crate::entities::placed_environment_volumes(&document) {
            if let Some(model) = volume.model {
                let key = super::super::physics_shape_key(&model, 4, [1.; 3], 0., false).unwrap();
                paths.insert(super::super::physics_shape_asset_path(&key));
            }
        }
    }
    let mut counts = [0usize; 4];
    for path in paths {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let world = super::super::parse_hkx(&bytes).unwrap();
        counts[0] += 1;
        for (index, item) in world.items.iter().enumerate() {
            let kind = &world.types[item.type_index].name;
            if kind != "hknpConvexPolytopeShape" && kind != "hknpBoxShape" {
                continue;
            }
            let shape = world
                .convex_polytope(&bytes, index)
                .unwrap_or_else(|e| panic!("{path} shape {index}: {e}"));
            counts[1] += 1;
            let connected = shape.connectivity.is_some();
            if connected {
                counts[2] += 1;
            }
            for vertex in shape.indices.iter().copied().collect::<BTreeSet<_>>() {
                let p = shape.vertices[usize::from(vertex)];
                let query = crate::gameplay::polytope_query::zero_distance_point_query(
                    &shape,
                    [p[0], p[1], p[2]],
                )
                .unwrap_or_else(|e| panic!("{path} shape {index}: {e}"));
                assert_eq!(
                    query.is_some(),
                    connected && shape.radius == 0.,
                    "{path} shape {index}"
                );
                if connected {
                    let hint = crate::gameplay::polytope_query::vertex_edge_hint(
                        &shape,
                        usize::from(vertex),
                        [p[0], p[1], p[2]],
                    )
                    .unwrap_or_else(|e| panic!("{path} shape {index}: {e}"));
                    assert_eq!(hint, 0., "{path} shape {index}");
                }
                counts[3] += 1;
            }
        }
    }
    assert!(counts.iter().all(|&count| count > 0));
    println!("Environment trigger files/polytopes/connected/vertices={counts:?}");
}

#[test]
#[ignore = "requires installed physics archives; validates all level polytope query topology"]
fn installed_polytopes_keep_query_topology_and_radius() {
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut paths = BTreeSet::new();
    for world in catalog.paths_with_suffix(".bwm") {
        if let Some(physics) = LevelResources::resolve(&catalog, &world).unwrap().physics {
            paths.insert(physics);
        }
    }
    let mut counts = [0usize; 4];
    let mut radii = BTreeMap::new();
    for path in paths {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let world = super::super::parse(&bytes).unwrap();
        counts[0] += 1;
        for (index, item) in world.items.iter().enumerate() {
            let kind = &world.types[item.type_index].name;
            if kind != "hknpConvexPolytopeShape" && kind != "hknpBoxShape" {
                continue;
            }
            let shape = world
                .convex_polytope(&bytes, index)
                .unwrap_or_else(|e| panic!("{path} shape {index}: {e}"));
            counts[1] += 1;
            *radii.entry(shape.radius.to_bits()).or_insert(0usize) += 1;
            if shape.connectivity.is_none() {
                continue;
            }
            counts[2] += 1;
            let vertices: BTreeSet<_> = shape.indices.iter().copied().collect();
            for vertex in vertices {
                let p = shape.vertices[usize::from(vertex)];
                let hint = crate::gameplay::polytope_query::vertex_edge_hint(
                    &shape,
                    usize::from(vertex),
                    [p[0], p[1], p[2]],
                )
                .unwrap_or_else(|e| panic!("{path} shape {index} vertex {vertex}: {e}"));
                assert_eq!(hint, 0., "{path} shape {index} vertex {vertex}");
                counts[3] += 1;
            }
        }
    }
    assert!(counts.iter().all(|&count| count > 0), "{counts:?}");
    println!(
        "Installed polytope levels/shapes/connected/vertex-rings={counts:?}; radius bits={radii:?}"
    );
}
