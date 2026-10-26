use super::*;
#[test]
fn mass_tensor_matches_executed_conversion() {
    let mut count = 0;
    for (case, line) in include_str!("../../../../tests/fixtures/mass_tensor.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let principal = PrincipalMassProperties {
            volume: f32::from_bits(w[0]),
            mass: f32::from_bits(w[1]),
            center: vector(2),
            inertia: vector(6),
            orientation: vector(10),
        };
        let tensor = principal.to_tensor();
        let actual: Vec<u32> = [tensor.volume, tensor.mass]
            .into_iter()
            .chain(tensor.center)
            .chain(tensor.inertia.into_iter().flatten())
            .map(f32::to_bits)
            .collect();
        assert_eq!(actual, w[14..], "case {case}");
        count += 1;
    }
    assert_eq!(count, 1024);
}
