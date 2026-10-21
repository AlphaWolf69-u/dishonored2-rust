//! Grid strips (1403026c0) and LOD edge fans (140301dd0).

fn vertex(grid_size: i32, x: i64, y: i64) -> u16 {
    // The source emits 16-bit indices, including truncation of grid addressing.
    (y * (i64::from(grid_size) + 1) + x) as u16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OceanGeometryRecord {
    pub vertex_count: u32,
    pub strip_primitive_count: u32,
    pub strip_index_offset: u32,
    pub edge_vertex_count: u32,
    pub edge_triangle_count: u32,
    pub edge_index_offset: u32,
}

pub struct OceanGeometry {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    /// LOD-major, then left/right/bottom/top edge density in radix three.
    pub records: Vec<OceanGeometryRecord>,
}

/// CPU data prepared by 140301780 for the shared ocean vertex/index buffers.
/// These grid sizes address the source's nine-entry allocation-size table.
pub fn build_ocean_geometry(grid_size: u32) -> Result<OceanGeometry, String> {
    if !grid_size.is_power_of_two() || !(2..=256).contains(&grid_size) {
        return Err("ocean grid size must be a power of two from 2 through 256".into());
    }
    let mut mesh = OceanGeometry {
        vertices: (0..=grid_size)
            .flat_map(|y| (0..=grid_size).map(move |x| [x as f32, y as f32]))
            .collect(),
        indices: Vec::new(),
        records: Vec::new(),
    };
    let mut side = grid_size as i32;
    while side > 2 {
        let vertex_count = ((side + 1) * (side + 1)) as u32;
        for left in [side, side / 2, side / 4] {
            for right in [side, side / 2, side / 4] {
                for bottom in [side, side / 2, side / 4] {
                    for top in [side, side / 2, side / 4] {
                        let segments =
                            [left, right, bottom, top].map(|v| if v == side { 0 } else { v });
                        let rect = [
                            i32::from(left != side),
                            i32::from(bottom != side),
                            side - i32::from(right != side),
                            side - i32::from(top != side),
                        ];
                        let strip_index_offset = mesh.indices.len() as u32;
                        let strip_count =
                            append_ocean_strip(&mut mesh.indices, grid_size as i32, rect);
                        let edge_index_offset = mesh.indices.len() as u32;
                        let edge_count = append_ocean_edges(
                            &mut mesh.indices,
                            grid_size as i32,
                            [0, 0, side, side],
                            segments,
                        )?;
                        mesh.records.push(OceanGeometryRecord {
                            vertex_count,
                            strip_primitive_count: strip_count as u32 - 2,
                            strip_index_offset,
                            edge_vertex_count: vertex_count,
                            edge_triangle_count: edge_count as u32 / 3,
                            edge_index_offset,
                        });
                    }
                }
            }
        }
        side /= 2;
    }
    Ok(mesh)
}

/// Append a row-pair strip with degenerate connectors between consecutive rows.
/// Rectangle coordinates are [min_x, min_y, max_x, max_y]. Returns index count.
pub fn append_ocean_strip(indices: &mut Vec<u16>, grid_size: i32, rect: [i32; 4]) -> usize {
    let start = indices.len();
    let [x0, y0, x1, y1] = rect.map(i64::from);
    for y in y0..y1 {
        indices.extend([vertex(grid_size, x0, y), vertex(grid_size, x0, y + 1)]);
        for x in x0 + 1..=x1 {
            indices.extend([vertex(grid_size, x, y), vertex(grid_size, x, y + 1)]);
        }
        if y < y1 - 1 {
            indices.extend([
                vertex(grid_size, x0.max(x1), y + 1),
                vertex(grid_size, x0, y + 1),
            ]);
        }
    }
    indices.len() - start
}

/// Append edge triangle lists in bottom/right/top/left order. Segment counts
/// are supplied as [left, right, bottom, top]; nonpositive counts disable edges.
/// Shared corner triangles are omitted according to the adjacent active edge.
pub fn append_ocean_edges(
    indices: &mut Vec<u16>,
    grid_size: i32,
    rect: [i32; 4],
    segments: [i32; 4],
) -> Result<usize, String> {
    let start = indices.len();
    let [x0, y0, x1, y1] = rect.map(i64::from);
    let [left, right, bottom, top] = segments;
    let edges = [
        (bottom, x1 - x0, left, right),
        (right, y1 - y0, bottom, top),
        (top, x1 - x0, left, right),
        (left, y1 - y0, bottom, top),
    ];
    for (count, length, _, _) in edges {
        if count > 0 && length > 0 && length / i64::from(count) == 0 {
            return Err("ocean edge has more segments than grid intervals".into());
        }
    }
    for (edge, (count, length, adjacent_start, adjacent_end)) in edges.into_iter().enumerate() {
        if count <= 0 || length <= 0 {
            continue;
        }
        let step = length / i64::from(count);
        let half = step / 2;
        let point = |t: i64, inner: bool| {
            let inward = i64::from(inner);
            let (x, y) = match edge {
                0 => (x0 + t, y0 + inward),
                1 => (x1 - inward, y0 + t),
                2 => (x0 + t, y1 - inward),
                _ => (x0 + inward, y0 + t),
            };
            vertex(grid_size, x, y)
        };
        let mut offset = 0;
        while offset < length {
            let a = point(offset, false);
            let b = point(offset + step, false);
            let middle = point(offset + half, true);
            indices.extend(if edge < 2 {
                [a, middle, b]
            } else {
                [a, b, middle]
            });
            for i in 0..step {
                if (i < half && offset == 0 && i == 0 && adjacent_start > 0)
                    || (i >= half && offset == length - step && i == step - 1 && adjacent_end > 0)
                {
                    continue;
                }
                let outer = if i < half { a } else { b };
                let p = point(offset + i, true);
                let q = point(offset + i + 1, true);
                indices.extend(if edge < 2 {
                    [outer, p, q]
                } else {
                    [outer, q, p]
                });
            }
            offset += step;
        }
    }
    Ok(indices.len() - start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocean_geometry_build_matches_executed_initialization() {
        fn take<'a>(data: &mut &'a [u8], len: usize) -> &'a [u8] {
            let (value, tail) = data.split_at(len);
            *data = tail;
            value
        }
        fn word(data: &mut &[u8]) -> u32 {
            u32::from_le_bytes(take(data, 4).try_into().unwrap())
        }
        let mut data =
            include_bytes!("../../../tests/fixtures/ocean_geometry_build.bin").as_slice();
        let mut count = 0;
        while !data.is_empty() {
            let grid = word(&mut data);
            let vertices = word(&mut data) as usize;
            let indices = word(&mut data) as usize;
            let records = word(&mut data) as usize;
            let mesh = build_ocean_geometry(grid).unwrap();
            assert_eq!(mesh.vertices.len(), vertices, "grid {grid} vertex count");
            for vertex in mesh.vertices {
                assert_eq!(vertex.map(f32::to_bits), [word(&mut data), word(&mut data)]);
            }
            let expected = take(&mut data, indices * 2)
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(mesh.indices, expected, "grid {grid} indices");
            assert_eq!(mesh.records.len(), records, "grid {grid} record count");
            for record in mesh.records {
                let actual = [
                    record.vertex_count,
                    record.strip_primitive_count,
                    record.strip_index_offset,
                    record.edge_vertex_count,
                    record.edge_triangle_count,
                    record.edge_index_offset,
                ];
                assert_eq!(
                    actual,
                    std::array::from_fn(|_| word(&mut data)),
                    "grid {grid} record"
                );
            }
            count += 1;
        }
        assert_eq!(count, 4);
    }

    #[test]
    fn ocean_mesh_indices_match_executed_generators() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/ocean_mesh_indices.txt").lines() {
            let (input, expected) = line.split_once(" : ").unwrap();
            let (kind, input) = input.split_once(' ').unwrap();
            let words = input
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap() as i32)
                .collect::<Vec<_>>();
            assert_eq!(words.len(), 9);
            let rect = words[1..5].try_into().unwrap();
            let mut actual = vec![0x1234, 0xabcd];
            let count = match kind {
                "strip" => append_ocean_strip(&mut actual, words[0], rect),
                "edges" => {
                    append_ocean_edges(&mut actual, words[0], rect, words[5..9].try_into().unwrap())
                        .unwrap()
                }
                _ => panic!("unexpected generator {kind}"),
            };
            let expected = expected
                .split_whitespace()
                .map(|v| u16::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(&actual[..2], &[0x1234, 0xabcd]);
            assert_eq!(count, expected.len(), "case {cases} count");
            assert_eq!(&actual[2..], expected.as_slice(), "case {cases} {input}");
            cases += 1;
        }
        assert_eq!(cases, 1257);
    }
}
