use super::*;
use crate::gameplay::component_state::{
    declaration::DeclarationScheduling, entity::ComponentDeclaration,
};

fn snapshot(classes: &ClassHierarchy) -> Vec<u32> {
    let mut out = vec![
        classes.type_bits() as u32,
        u32::from(classes.is_initialized()),
        classes.by_name().len() as u32,
    ];
    out.extend(classes.by_name().iter().map(|&v| v as u32));
    out.push(classes.by_id().len() as u32);
    out.extend(classes.by_id().iter().map(|&v| v as u32));
    out.push(classes.roots().len() as u32);
    out.extend(classes.roots().iter().map(|&v| v as u32));
    for (index, node) in classes.nodes.iter().enumerate() {
        let children = classes.children(index).unwrap();
        out.extend([
            node.type_id as u32,
            node.last_type_id as u32,
            u32::from(node.initialized),
            children.len() as u32,
        ]);
        out.extend(children.iter().map(|&v| v as u32));
    }
    out
}

#[test]
fn hierarchy_ids_and_declaration_order_match_complete_executed_initialization() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/class_hierarchy.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap();
        let order = (0..count).map(|_| words.next().unwrap() as usize).collect();
        let bases = (0..count)
            .map(|_| {
                let base = words.next().unwrap();
                (base != u32::MAX).then_some(base as usize)
            })
            .collect();
        let mut classes = ClassHierarchy::new(bases, order).unwrap();
        let early = words.next().unwrap();
        for _ in 0..early {
            classes
                .initialize_node(words.next().unwrap() as usize)
                .unwrap();
        }
        assert!(classes.initialize().unwrap());
        let before = snapshot(&classes);
        assert_eq!(before, words.collect::<Vec<_>>(), "case {cases}");
        assert!(!classes.initialize().unwrap());
        assert_eq!(snapshot(&classes), before);
        for index in 0..count as usize {
            let declaration = ComponentDeclaration::from_class(
                index,
                index,
                &classes,
                DeclarationScheduling::from_base_flags(0),
            )
            .unwrap();
            assert_eq!(declaration.class_id, classes.node(index).unwrap().type_id);
        }
        cases += 1;
    }
    assert_eq!(cases, 512);
}
