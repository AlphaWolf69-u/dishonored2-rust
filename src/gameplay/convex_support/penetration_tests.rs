use super::penetration::*;
use crate::gameplay::shape_transform::inverse_rotate_vector;

#[test]
fn penetration_support_matches_executed_helpers() {
    fn words(line: &str) -> Vec<u32> {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect()
    }
    let inputs: Vec<_> = include_str!("../../../tests/fixtures/convex_support_vertices.txt")
        .lines()
        .map(words)
        .collect();
    for line in include_str!("../../../tests/fixtures/penetration_support.txt").lines() {
        let w = words(line);
        let case = w[0] as usize;
        let input = &inputs[case];
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(input[at + i]));
        let ac = input[0] as usize;
        let bc = input[1] as usize;
        let a: Vec<_> = (0..ac).map(|i| vector(22 + i * 4)).collect();
        let b: Vec<_> = (0..bc).map(|i| vector(22 + (ac + i) * 4)).collect();
        let relative = std::array::from_fn(|i| vector(6 + i * 4));
        let source = PenetrationSupport {
            query: &a,
            target: &b,
            relative,
        };
        let point = source.point(vector(2)).unwrap();
        let (extent, best) = source
            .extent(vector(2), std::array::from_fn(|i| f32::from_bits(w[1 + i])))
            .unwrap();
        let result: Vec<_> = point
            .difference
            .into_iter()
            .chain(point.query)
            .chain(point.target)
            .chain([extent])
            .chain(best.difference)
            .chain(best.query)
            .chain(best.target)
            .chain(inverse_rotate_vector(relative, vector(2)))
            .map(f32::to_bits)
            .collect();
        assert_eq!(result, w[5..], "penetration support case{case}");
    }
}
