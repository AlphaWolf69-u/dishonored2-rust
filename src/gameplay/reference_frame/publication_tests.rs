use super::*;

#[test]
fn publications_match_executed_frame_updates() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/reference_frame_publication.txt").lines() {
        let words: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let vector = |at| std::array::from_fn(|i| f32::from_bits(words[at + i]));
        let frame = |at| ReferenceFrame {
            position: vector(at),
            basis: std::array::from_fn(|i| vector(at + 3 + i * 3)),
        };
        let mut cache = ReferenceFrameCache {
            current: frame(0),
            scale: vector(12),
            published_velocities: [vector(15), vector(18)],
            marker: words[21] as i32,
            predicted: frame(22),
        };
        cache.publish(frame(34), [vector(46), vector(49)], words[52] as i32);
        let mut actual = Vec::new();
        actual.extend(cache.current.position.map(f32::to_bits));
        actual.extend(cache.current.basis.into_iter().flatten().map(f32::to_bits));
        actual.extend(cache.scale.map(f32::to_bits));
        actual.extend(
            cache
                .published_velocities
                .into_iter()
                .flatten()
                .map(f32::to_bits),
        );
        actual.push(cache.marker as u32);
        actual.extend(cache.predicted.position.map(f32::to_bits));
        actual.extend(
            cache
                .predicted
                .basis
                .into_iter()
                .flatten()
                .map(f32::to_bits),
        );
        assert_eq!(actual, words[53..], "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}
