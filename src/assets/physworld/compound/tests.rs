use crate::gameplay::compound_query::{
    CompoundQuery, CompoundQueryBounds, CompoundQueryVisitor, CompoundTree,
};
use crate::{archive::Catalog, assets::level::LevelResources};
use std::collections::BTreeSet;

#[test]
#[ignore = "requires installed level physics archives"]
fn installed_compounds_keep_query_tree_and_instances() {
    struct Visitor(Vec<u16>);
    impl CompoundQueryVisitor for Visitor {
        fn query_instance(&mut self, instance: u16, _: &mut f32) {
            self.0.push(instance);
        }
    }
    let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut paths = BTreeSet::new();
    for path in catalog.paths_with_suffix(".bwm") {
        if let Some(physics) = LevelResources::resolve(&catalog, &path).unwrap().physics {
            paths.insert(physics);
        }
    }
    let mut counts = [0usize; 5];
    let mut mutable_shapes = 0usize;
    let mut traversal = CompoundQuery::default();
    for path in paths {
        let bytes = catalog.read(&path).unwrap().unwrap();
        let world = super::super::parse(&bytes).unwrap();
        counts[0] += 1;
        for (index, item) in world.items.iter().enumerate() {
            if world.types[item.type_index].name != "hknpCompoundShape" {
                continue;
            }
            let compound = world
                .compound_shape(&bytes, index)
                .unwrap_or_else(|e| panic!("{path} shape {index}: {e}"));
            assert!(compound.key_bits <= 32);
            mutable_shapes += usize::from(compound.is_mutable);
            // A compound key must distinguish every reachable instance. Reading
            // runtime offset23 from a serialized header incorrectly gives zero.
            counts[1] += 1;
            counts[2] += compound.instances.len();
            counts[3] += compound.nodes.len();
            let mut visitor = Visitor(Vec::new());
            let mut distance = f32::INFINITY;
            traversal
                .visit(
                    CompoundTree {
                        nodes: &compound.nodes,
                        root: compound.root,
                    },
                    CompoundQueryBounds {
                        center: [0.; 3],
                        half_extent: [0.; 3],
                    },
                    &mut distance,
                    &mut visitor,
                )
                .unwrap();
            assert_eq!(
                visitor.0.len(),
                visitor.0.iter().collect::<BTreeSet<_>>().len(),
                "{path} shape {index}"
            );
            for &leaf in &visitor.0 {
                assert!(
                    u64::from(leaf) < (1u64 << compound.key_bits),
                    "{path} shape {index}: instance {leaf} exceeds key width {}",
                    compound.key_bits
                );
                let instance = &compound.instances[usize::from(leaf)];
                if instance.flags & 64 != 0 {
                    assert!(
                        instance.shape != 0 && instance.shape < world.items.len(),
                        "{path} shape {index} instance {leaf}"
                    );
                }
            }
            counts[4] += visitor.0.len();
        }
    }
    assert!(counts.iter().all(|&count| count > 0));
    println!("Compound levels/shapes/instances/nodes/live leaves={counts:?}");
    println!("Mutable compound shapes={mutable_shapes}");
}
