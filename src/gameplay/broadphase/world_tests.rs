use super::*;

#[test]
fn constructor_and_pending_body_marks_match_executed_hybrid() {
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/broadphase_updates.txt").lines() {
        let mut values = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let capacity = values.next().unwrap() as usize;
        let mut world = HybridBroadphase::with_body_capacity(capacity);
        let n = values.next().unwrap() as usize;
        world.mark_bodies(values.by_ref().take(n));
        let bits = values.next().unwrap() as usize;
        let word_count = values.next().unwrap() as usize;
        let n = values.next().unwrap() as usize;
        let mut words = vec![0; word_count];
        for _ in 0..n {
            let index = values.next().unwrap() as usize;
            words[index] = values.next().unwrap();
        }
        let range = [values.next().unwrap(), values.next().unwrap()];
        let n = values.next().unwrap() as usize;
        let pairs: Vec<_> = (0..n)
            .map(|_| [values.next().unwrap() as u8, values.next().unwrap() as u8])
            .collect();
        assert_eq!(world.pending_body_bits().len(), bits, "case {count}");
        assert_eq!(world.pending_body_bits().words(), words, "case {count}");
        assert_eq!(
            world.pending_body_range().unwrap_or([u32::MAX, 0]),
            range,
            "case {count}"
        );
        assert_eq!(
            world.layer_pairs(),
            pairs,
            "case {count}: configured pair order"
        );
        assert!(values.next().is_none());
        count += 1;
    }
    assert_eq!(count, 512);
}

#[test]
fn five_layer_world_matches_executed_manager() {
    use super::super::tests::{assert_tree_matches, body};
    for (case, line) in include_str!("../../../tests/fixtures/hybrid_world_build.txt")
        .lines()
        .enumerate()
    {
        let mut w = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let mode = w.next().unwrap();
        let count = w.next().unwrap() as usize;
        assert!(w.next().unwrap() < 2);
        let bodies: Vec<_> = (0..count)
            .map(|_| BroadphaseBody {
                body: body(&mut w),
                flags: w.next().unwrap(),
                status: w.next().unwrap() as u16,
            })
            .collect();
        let mut world = HybridBroadphase::default();
        let mut locations = vec![u32::MAX; count];
        let mut publish = |id: u32, location: u32| locations[(id & 0xff_ffff) as usize] = location;
        if mode == 0 {
            world.add_bodies(&bodies, &mut publish).unwrap();
        } else {
            let step = if mode == 1 { 1 } else { 17 };
            for chunk in bodies.chunks(step) {
                world.add_bodies(chunk, &mut publish).unwrap();
            }
        }
        let views = world.query_trees();
        for (i, tree) in world.trees.iter().enumerate() {
            assert_eq!(tree.root, views[i].root);
            assert_tree_matches(tree, &locations, &mut w, &format!("{case} layer {i}"));
        }
        for flag in world.dirty_layers() {
            assert_eq!(*flag, w.next().unwrap() != 0, "case {case} dirty flags");
        }
        assert_eq!(w.next(), None);
    }
}
#[test]
fn layer_settings_and_classification_match_executed_configuration() {
    let mut lines = include_str!("../../../tests/fixtures/broadphase_layers.txt").lines();
    let mut header = lines
        .next()
        .unwrap()
        .split_whitespace()
        .map(|v| u32::from_str_radix(v, 16).unwrap());
    let world = HybridBroadphase::default();
    assert_eq!(
        world.layer_settings().len(),
        header.next().unwrap() as usize
    );
    for layer in world.layer_settings() {
        assert_eq!(layer.collision_mask, header.next().unwrap());
        assert_eq!(
            layer.flags,
            [header.next().unwrap() as u8, header.next().unwrap() as u8]
        );
    }
    assert_eq!(header.next(), None);
    for (case, line) in lines.enumerate() {
        let v: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        assert_eq!(body_layer(v[0], v[1] as u16), v[2] as usize, "case {case}");
    }
}
