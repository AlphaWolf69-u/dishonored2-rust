use super::*;
use crate::assets::{
    entities::{declaration_text, parse, triggers::TriggerAssets},
    level::LevelResources,
};

#[test]
#[ignore = "requires installed level and trigger archives"]
fn installed_trigger_query_geometry_is_retained() {
    let catalog =
        crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut counts = [0usize; 5];
    for path in catalog.paths_with_suffix(".bwm") {
        let level = LevelResources::resolve(&catalog, &path).unwrap();
        let bytes = catalog.read(&level.entities).unwrap().unwrap();
        let document = parse(&declaration_text(&bytes)).unwrap();
        let map = format!(
            "{}.map",
            level
                .entities
                .strip_prefix("maps/")
                .unwrap()
                .strip_suffix(".entities")
                .unwrap()
        );
        let assets = TriggerAssets::load(&document, &map, &catalog).unwrap();
        counts[0] += 1;
        counts[1] += assets.shapes.len();
        for asset in &assets.shapes {
            assert_eq!(asset.query_shapes.root(), 1);
            for (_, shape) in asset.query_shapes.iter() {
                counts[2] += 1;
                match &shape.geometry {
                    QueryGeometry::Compound(compound) => {
                        for instance in &compound.instances {
                            if instance.flags & 64 != 0 {
                                asset.query_shapes.get(instance.shape).unwrap();
                                counts[3] += 1;
                            }
                        }
                    }
                    QueryGeometry::Polytope(polytope) => {
                        counts[4] += usize::from(polytope.connectivity.is_none());
                    }
                }
            }
        }
    }
    assert_eq!(counts, [36, 5576, 20559, 14983, 4]);
    println!("worlds/resources/shapes/live-instances/no-connectivity={counts:?}");
}
