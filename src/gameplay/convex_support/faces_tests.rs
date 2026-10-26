use super::faces::*;
#[test]
fn penetration_faces_match_executed_geometry_and_topology() {
    for (case, line) in include_str!("../../../tests/fixtures/penetration_faces.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let points = std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[i * 4 + j])));
        let (normal, distance) = face_plane([points[0], points[1], points[2]]);
        let mut result: Vec<_> = normal
            .into_iter()
            .chain([distance])
            .map(f32::to_bits)
            .collect();
        for face in tetrahedron_faces(points) {
            result.extend(face.vertices.map(|i| i as u32));
            result.extend(
                face.opposite
                    .into_iter()
                    .flat_map(|edge| [edge.face as u32, edge.edge as u32]),
            );
            result.extend(
                face.normal
                    .into_iter()
                    .chain([face.distance])
                    .map(f32::to_bits),
            );
            result.push(face.mark);
        }
        assert_eq!(result, w[16..], "penetration face case{case}");
    }
}
