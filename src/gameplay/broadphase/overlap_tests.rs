use super::*;

#[test]
fn ordered_overlaps_match_executed_five_layer_query() {
    let mut world = HybridBroadphase::default();
    let mut quantization =
        BroadphaseQuantization::new([[-1000., -1000., -1000., 0.], [1000., 1000., 1000., 0.]]);
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/hybrid_overlap.txt").lines() {
        let mut parts = row.split_whitespace();
        let kind = parts.next().unwrap();
        let mut words = parts.map(|v| u32::from_str_radix(v, 16).unwrap());
        if kind == "initial" {
            let n = words.next().unwrap();
            let axis = words.next().unwrap();
            let z = if axis != 0 { 500. } else { 1000. };
            quantization =
                BroadphaseQuantization::new([[-1000., -1000., -z, 0.], [1000., 1000., z, 0.]]);
            world = HybridBroadphase::default();
            let bodies: Vec<_> = (0..n)
                .map(|_| {
                    let bounds = QuantizedBounds {
                        min: std::array::from_fn(|_| words.next().unwrap() as u16),
                        max: std::array::from_fn(|_| words.next().unwrap() as u16),
                    };
                    BroadphaseBody {
                        body: PointQueryBody {
                            bounds,
                            index: words.next().unwrap(),
                        },
                        flags: words.next().unwrap(),
                        status: words.next().unwrap() as u16,
                    }
                })
                .collect();
            world.add_bodies(&bodies, |_, _| {}).unwrap();
            assert!(words.next().is_none());
        } else {
            let packed = PackedBodyBounds {
                words: std::array::from_fn(|_| words.next().unwrap()),
            };
            let n = words.next().unwrap() as usize;
            let expected: Vec<_> = words.collect();
            assert_eq!(expected.len(), n);
            let mut actual = vec![0x1234];
            world
                .overlap_bodies(packed, &quantization.packed, &mut actual)
                .unwrap();
            assert_eq!(actual[0], 0x1234);
            assert_eq!(actual[1..], expected, "query {count}");
            count += 1;
        }
    }
    assert_eq!(count, 512);
}
