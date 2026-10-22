use super::*;

#[test]
fn closest_point_tables_and_selection_match_executed_dispatcher() {
    let words = |line: &str| {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>()
    };
    let mut current = None;
    let mut table = None;
    let mut pairs = 0;
    for line in include_str!("../../../tests/fixtures/closest_point_dispatch.txt").lines() {
        let v = words(line);
        assert_eq!(v.len(), 7);
        if current != Some(v[0]) {
            table = Some(ClosestPointDispatch::new(v[0]).unwrap());
            current = Some(v[0]);
        }
        let e = table
            .as_ref()
            .unwrap()
            .entry(v[1] as u8, v[2] as u8)
            .unwrap();
        assert_eq!(
            [
                e.handler as u32,
                e.depth as u32,
                e.registered_query as u32,
                e.registered_target as u32
            ],
            v[3..],
            "mask{:x} pair{}:{}",
            v[0],
            v[1],
            v[2]
        );
        pairs += 1;
    }
    assert_eq!(pairs, 3750);
    let table = ClosestPointDispatch::new(u32::MAX).unwrap();
    let mut selected = 0;
    for line in include_str!("../../../tests/fixtures/closest_point_selection.txt").lines() {
        let v = words(line);
        assert_eq!(v.len(), 9);
        let filter = (v[2] != 0).then_some(CollisionQueryFilter {
            layers: v[3],
            required_flags: v[4],
        });
        assert_eq!(
            table
                .select(v[0] as u8, v[1] as u8, filter, [v[5], v[6]], v[7] != 0)
                .unwrap() as u32,
            v[8],
            "selection{selected}"
        );
        selected += 1;
    }
    assert_eq!(selected, 3750);
}
