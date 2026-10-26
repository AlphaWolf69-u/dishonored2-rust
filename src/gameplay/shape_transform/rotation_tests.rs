use super::*;
#[test]
fn retry_rotation_matches_executed_helpers() {
    for (case, line) in include_str!("../../../tests/fixtures/shape_rotation.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let v = |offset| std::array::from_fn(|i| f32::from_bits(w[offset + i]));
        let angles = v(0);
        let axis = v(4);
        let mut matrix = std::array::from_fn(|i| v(8 + i * 4));
        let (sine, cosine) = sin_cos(angles);
        let quaternion = axis_angle_quaternion(axis, angles[0]);
        let basis = axis_angle_basis(axis, angles[0]);
        compose_basis(&mut matrix, basis);
        let actual: Vec<u32> = sine
            .into_iter()
            .chain(cosine)
            .chain(quaternion)
            .chain(basis.into_iter().flatten())
            .chain(matrix.into_iter().flatten())
            .map(f32::to_bits)
            .collect();
        assert_eq!(actual.len(), w.len() - 24);
        for (lane, (&got, &expected)) in actual.iter().zip(&w[24..]).enumerate() {
            assert_eq!(
                got, expected,
                "case {case} lane {lane}: {got:08x} != {expected:08x}"
            );
        }
    }
}
