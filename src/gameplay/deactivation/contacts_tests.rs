use super::*;

#[test]
fn deactivation_contacts_match_executed_pair_and_swap_order() {
    let mut cases = 0;
    for row in include_str!("../../../tests/fixtures/deactivation_contacts.txt").lines() {
        let mut w = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap());
        let case = w.next().unwrap();
        let mut bodies = BodyStorage::new(8).unwrap();
        for slot in 1..8 {
            let id = bodies.reserve(&mut []).unwrap();
            let body = bodies.get_mut(id).unwrap();
            body.motion_id = w.next().unwrap();
            body.flags = w.next().unwrap();
            body.bounds.words = [slot, 0, 0, 0];
        }
        let lists: Vec<Vec<u32>> = (0..7)
            .map(|_| {
                let n = w.next().unwrap() as usize;
                w.by_ref().take(n).collect()
            })
            .collect();
        let n = w.next().unwrap() as usize;
        let ids: Vec<_> = w.by_ref().take(n).collect();
        let mut pairs = vec![[123, 456], [789, 10]];
        let mut observed = Vec::new();
        let result = prepare_contacts(&bodies, &ids, &mut pairs, |bounds, hits| {
            let slot = bounds.words[0];
            observed.push(slot);
            hits.extend_from_slice(&lists[slot as usize - 1]);
            Ok(())
        })
        .unwrap();
        assert_eq!(result, w.next().unwrap() != 0, "result case{case}");
        let n = w.next().unwrap() as usize;
        assert_eq!(
            pairs.into_iter().flatten().collect::<Vec<_>>(),
            w.by_ref().take(n * 2).collect::<Vec<_>>(),
            "pairs case{case}"
        );
        let n = w.next().unwrap() as usize;
        assert_eq!(
            observed,
            w.by_ref().take(n).collect::<Vec<_>>(),
            "queries case{case}"
        );
        assert_eq!(w.next(), None);
        cases += 1;
    }
    assert_eq!(cases, 512);
}
