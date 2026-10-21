//! Finite wave collision-height raster mesh from 1402fa4e0.

pub struct WaveCollisionMesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

/// The wave factory passes simulation_resolution/4 (128). The extra 48 cells
/// on each side allow displaced triangles to cover the collision target edges.
pub fn build_wave_collision_mesh(resolution: u32) -> Result<WaveCollisionMesh, String> {
    if resolution < 2 {
        return Err("wave collision resolution must exceed one".into());
    }
    let width = resolution
        .checked_add(96)
        .ok_or("wave collision width overflow")?;
    let vertex_count = width
        .checked_mul(width)
        .ok_or("wave collision vertex count overflow")?;
    let index_count = (u64::from(width) + 1) * u64::from(width - 1) * 2 - 2;
    if index_count > u64::from(u32::MAX) {
        return Err("wave collision index count overflow".into());
    }
    let mut vertices = Vec::with_capacity(vertex_count as usize);
    for y in 0..width {
        for x in 0..width {
            vertices.push([
                (x as f32 - 48.) / (resolution - 1) as f32,
                (y as f32 - 48.) / (resolution - 1) as f32,
            ]);
        }
    }
    let mut indices = Vec::with_capacity(index_count as usize);
    let mut next = 0;
    for y in 0..width - 1 {
        for _ in 0..width {
            indices.extend([next, next + width]);
            next += 1;
        }
        if y < width - 2 {
            indices.extend([next + width - 1, next]);
        }
    }
    Ok(WaveCollisionMesh { vertices, indices })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collision_mesh_matches_executed_generator() {
        fn digest(bytes: impl Iterator<Item = u8>) -> u64 {
            bytes.fold(0xcbf2_9ce4_8422_2325, |value, byte| {
                (value ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
            })
        }
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/ocean_collision_mesh.txt").lines() {
            let row = line
                .split_whitespace()
                .map(|v| u64::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let mesh = build_wave_collision_mesh(row[0] as u32).unwrap();
            assert_eq!(mesh.indices.len() as u64, row[1]);
            assert_eq!(mesh.vertices.len() as u64, row[2]);
            assert_eq!(
                digest(mesh.indices.into_iter().flat_map(u32::to_le_bytes)),
                row[3]
            );
            assert_eq!(
                digest(
                    mesh.vertices
                        .into_iter()
                        .flatten()
                        .flat_map(f32::to_le_bytes)
                ),
                row[4]
            );
            cases += 1;
        }
        assert_eq!(cases, 3);
    }
}
