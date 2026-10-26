use super::*;
#[test]
fn point_shape_admission_matches_executed_wrapper() {
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/point_shape_admission.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let vector = |at| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let actual =
            PointSphere::for_query(vector(0), f32::from_bits(w[4]), [vector(5), vector(9)]);
        assert_eq!(actual.is_some(), w[13] != 0, "admission {count}");
        if let Some(sphere) = actual {
            assert_eq!(sphere.vertex.map(f32::to_bits), w[14..18], "point {count}");
            assert_eq!(sphere.radius.to_bits(), 0);
        }
        count += 1;
    }
    assert_eq!(count, 1536);
}
#[test]
fn closest_point_preparation_matches_executed_dispatch_inputs() {
    for (case, line) in include_str!("../../../tests/fixtures/closest_point_preparation.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let matrix = |offset| {
            std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[offset + i * 4 + j])))
        };
        let mut limits = std::array::from_fn(|i| f32::from_bits(w[33 + i]));
        let relative =
            prepare_transforms(matrix(0), matrix(16), f32::from_bits(w[32]), &mut limits);
        let actual: Vec<u32> = relative
            .into_iter()
            .flatten()
            .chain(limits)
            .map(f32::to_bits)
            .collect();
        assert_eq!(actual, &w[37..], "case {case}");
    }
}
#[test]
fn point_sphere_bounds_match_executed_constructor_and_virtual() {
    use super::PointSphere;
    use crate::gameplay::compound_query::prepare::CompoundQueryGeometry;
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/point_sphere_bounds.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let vector = |at| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let mut sphere = PointSphere::new(vector(0), f32::from_bits(w[4]));
        assert_eq!(sphere.vertex.map(f32::to_bits), w[21..25], "vertex {count}");
        let bounds = sphere
            .calculate_bounds(std::array::from_fn(|i| vector(5 + 4 * i)))
            .unwrap();
        let actual: Vec<_> = bounds.into_iter().flatten().map(f32::to_bits).collect();
        assert_eq!(actual, w[25..33], "bounds {count}");
        count += 1;
    }
    assert_eq!(count, 1024);
}
