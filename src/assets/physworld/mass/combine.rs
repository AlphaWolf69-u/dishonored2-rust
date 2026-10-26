use super::{add_parallel_axis, rotate_inertia, MassProperties};
use crate::float::{add, multiply};

/// One c2bd80 input record: full mass tensor followed by its rigid transform.
#[derive(Clone, Copy, Debug)]
pub struct MassElement {
    pub properties: MassProperties,
    pub transform: [[f32; 4]; 4],
}
fn world_center(element: &MassElement) -> [f32; 4] {
    let c = element.properties.center;
    let t = element.transform;
    std::array::from_fn(|i| {
        add(
            add(
                add(multiply(c[1], t[1][i]), multiply(c[0], t[0][i])),
                multiply(c[2], t[2][i]),
            ),
            t[3][i],
        )
    })
}

/// c2bd80 combines in input order, retaining four COM lanes and twelve inertia
/// lanes. Nonpositive or unordered total mass leaves output untouched (status1).
pub fn combine_mass(elements: &[MassElement], output: &mut MassProperties) -> bool {
    let mut mass = 0.;
    let mut volume = 0.;
    let mut weighted = [0.; 4];
    for element in elements {
        mass = add(mass, element.properties.mass);
        volume = add(volume, element.properties.volume);
        let center = world_center(element);
        for i in 0..4 {
            weighted[i] = add(weighted[i], multiply(center[i], element.properties.mass));
        }
    }
    if !(mass > 0.) {
        return false;
    }
    let inverse_mass = 1. / mass;
    let center = weighted.map(|v| multiply(v, inverse_mass));
    let mut inertia = [[0.; 4]; 3];
    for element in elements {
        let mut tensor = element.properties.inertia;
        rotate_inertia(
            &mut tensor,
            [
                element.transform[0],
                element.transform[1],
                element.transform[2],
            ],
        );
        let position = world_center(element);
        let offset = std::array::from_fn(|i| position[i] - center[i]);
        add_parallel_axis(&mut tensor, offset, [element.properties.mass; 4]);
        for column in 0..3 {
            for row in 0..4 {
                inertia[column][row] = if column == 0 {
                    add(inertia[column][row], tensor[column][row])
                } else {
                    add(tensor[column][row], inertia[column][row])
                };
            }
        }
    }
    *output = MassProperties {
        volume,
        mass,
        center,
        inertia,
    };
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Words<'a>(std::str::SplitWhitespace<'a>);
    impl Words<'_> {
        fn word(&mut self) -> u32 {
            u32::from_str_radix(self.0.next().unwrap(), 16).unwrap()
        }
        fn float(&mut self) -> f32 {
            f32::from_bits(self.word())
        }
        fn vector(&mut self) -> [f32; 4] {
            std::array::from_fn(|_| self.float())
        }
        fn properties(&mut self) -> MassProperties {
            MassProperties {
                volume: self.float(),
                mass: self.float(),
                center: self.vector(),
                inertia: std::array::from_fn(|_| self.vector()),
            }
        }
    }
    fn words(p: MassProperties) -> Vec<u32> {
        [p.volume, p.mass]
            .into_iter()
            .chain(p.center)
            .chain(p.inertia.into_iter().flatten())
            .map(f32::to_bits)
            .collect()
    }
    #[test]
    fn combined_mass_matches_executed_routine() {
        let mut count = 0;
        for (case, line) in include_str!("../../../../tests/fixtures/combined_mass.txt")
            .lines()
            .enumerate()
        {
            let mut input = Words(line.split_whitespace());
            let n = input.word() as usize;
            let mut output = input.properties();
            let elements: Vec<_> = (0..n)
                .map(|_| MassElement {
                    properties: input.properties(),
                    transform: std::array::from_fn(|_| input.vector()),
                })
                .collect();
            let success = combine_mass(&elements, &mut output);
            let expected = input.properties();
            let status = input.word();
            assert_eq!(u32::from(!success), status, "case {case} status");
            assert_eq!(words(output), words(expected), "case {case} output");
            assert!(input.0.next().is_none());
            count += 1;
        }
        assert_eq!(count, 256);
    }
}
