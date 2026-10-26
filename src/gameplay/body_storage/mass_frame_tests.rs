use super::*;
use crate::assets::physworld::mass::PrincipalMassProperties;

#[test]
fn body_mass_frames_match_executed_manager() {
    let mut count = 0;
    for (case, line) in include_str!("../../../tests/fixtures/body_mass_frame.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let ident = w[0];
        let selector = w[1] as u8;
        let present = w[2] != 0;
        let allocated = w[3] != 0;
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let mut bodies = BodyStorage::new(2).unwrap();
        let id = bodies.reserve(&mut []).unwrap();
        let body = bodies.get_mut(id).unwrap();
        body.shape_slots = Some(std::array::from_fn(|i| i + 1));
        body.shape_selectors[1] = selector;
        if allocated {
            bodies.set_motion_properties(0, 1).unwrap();
            bodies.set_mass(id, f32::from_bits(w[4])).unwrap();
        }
        if present {
            bodies.mass_distributions.insert(
                1,
                MassDistribution {
                    center_of_mass_and_volume: vector(19),
                    major_axis_space: vector(23),
                    inertia_tensor: vector(27),
                },
            );
        }
        let mut observed = Vec::new();
        let frame = bodies
            .mass_frame(
                ident,
                |shape| {
                    assert_eq!(shape, usize::from(selector & 7) + 1);
                    Ok(None)
                },
                |shape, config| {
                    let index = shape - 1;
                    observed.extend([index as u32, config[0], config[1], config[2]]);
                    Ok(PrincipalMassProperties {
                        volume: f32::from_bits(w[5]),
                        mass: f32::from_bits(w[6]) + index as f32,
                        center: vector(7),
                        inertia: vector(11),
                        orientation: vector(15),
                    })
                },
            )
            .unwrap();
        let actual: Vec<u32> = [frame.volume, frame.mass]
            .into_iter()
            .chain(frame.center)
            .chain(frame.inertia)
            .chain(frame.orientation)
            .map(f32::to_bits)
            .collect();
        assert_eq!(actual, w[31..45], "case {case}");
        assert_eq!(observed.len(), w[45] as usize);
        assert_eq!(observed, w[46..], "case {case} selected fallback");
        count += 1;
    }
    assert_eq!(count, 1024);
}
