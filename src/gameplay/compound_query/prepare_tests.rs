use super::prepare::*;
use crate::gameplay::shape_transform::ScaledShapeTransform;

#[test]
fn compound_preparation_matches_executed_wrapper() {
    struct Geometry {
        vertices: [[f32; 4]; 3],
        radius: f32,
        convex: bool,
        bounds: [[f32; 4]; 2],
        matrices: Vec<[[f32; 4]; 4]>,
    }
    impl CompoundQueryGeometry for Geometry {
        fn calculate_bounds(&mut self, matrix: [[f32; 4]; 4]) -> Result<[[f32; 4]; 2], String> {
            self.matrices.push(matrix);
            Ok(self.bounds)
        }
        fn convex_vertices(&self) -> Option<(&[[f32; 4]], f32)> {
            self.convex.then_some((&self.vertices[..], self.radius))
        }
    }
    for (case, line) in include_str!("../../../tests/fixtures/compound_preparation.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let matrix = |at| std::array::from_fn(|i| vector(at + 4 * i));
        let query = CompoundQueryTransform {
            scaled: w[0] != 0,
            shape: ScaledShapeTransform {
                world_from_local: matrix(4),
                scale: vector(52),
                offset: vector(60),
            },
        };
        let target = CompoundQueryTransform {
            scaled: w[1] != 0,
            shape: ScaledShapeTransform {
                world_from_local: matrix(20),
                scale: vector(56),
                offset: vector(64),
            },
        };
        let mut geometry = Geometry {
            vertices: std::array::from_fn(|i| vector(70 + i * 4)),
            radius: f32::from_bits(w[68]),
            convex: w[2] != 0,
            bounds: [vector(82), vector(86)],
            matrices: Vec::new(),
        };
        let prepared = prepare(
            &mut geometry,
            query,
            target,
            matrix(36),
            f32::from_bits(w[69]),
        )
        .unwrap();
        let bits: Vec<_> = prepared
            .bounds
            .center
            .into_iter()
            .chain(prepared.bounds.half_extent)
            .chain([prepared.squared_distance])
            .map(f32::to_bits)
            .collect();
        assert_eq!(bits, w[90..97], "compound preparation case {case}");
        assert_eq!(
            geometry.matrices.len(),
            w[97] as usize,
            "bounds callback count case {case}"
        );
        if let Some(matrix) = geometry.matrices.first() {
            assert_eq!(
                matrix
                    .iter()
                    .flatten()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                w[98..114],
                "bounds callback matrix case {case}"
            );
        }
    }
}
