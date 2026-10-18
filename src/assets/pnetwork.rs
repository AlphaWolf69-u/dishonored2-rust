//! Reader for the ordered probe resource names at the start of a PNW network.
//! The game loader consumes this list in FUN_1402b15e0 before its spatial tables.

fn u32le(bytes: &[u8], at: usize) -> Result<u32, String> {
    let end = at
        .checked_add(4)
        .ok_or("probe-network integer offset overflow")?;
    let raw = bytes.get(at..end).ok_or_else(|| {
        format!(
            "truncated probe-network integer at {at:#x} of {:#x}",
            bytes.len()
        )
    })?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}

pub fn ordered_probe_names(bytes: &[u8]) -> Result<(u8, Vec<String>, Vec<String>, usize), String> {
    if bytes.len() < 8 || bytes.get(..3) != Some(b"PNW") {
        return Err("invalid or truncated PNW header".into());
    }
    let version = bytes[3];
    if version < 5 {
        return Err(format!("unsupported probe-network version {version}"));
    }
    let count = u32le(bytes, 4)? as usize;
    if count > 1_000_000 {
        return Err(format!("unreasonable probe-network count {count}"));
    }
    let mut cursor = 8usize;
    let mut names = Vec::with_capacity(count);
    for _ in 0..count {
        if version < 6 {
            cursor = cursor
                .checked_add(12)
                .ok_or("probe-network offset overflow")?;
        }
        let length = u32le(bytes, cursor)? as usize;
        cursor = cursor
            .checked_add(4)
            .ok_or("probe-network offset overflow")?;
        if length > 1 << 20 {
            return Err(format!("unreasonable probe resource name length {length}"));
        }
        let end = cursor
            .checked_add(length)
            .ok_or("probe-network offset overflow")?;
        let raw = bytes
            .get(cursor..end)
            .ok_or("truncated probe resource name")?;
        let name = std::str::from_utf8(raw).map_err(|error| error.to_string())?;
        names.push(name.trim_end_matches('\0').to_owned());
        cursor = end;
    }
    let secondary_count = u32le(bytes, cursor)? as usize;
    cursor += 4;
    if secondary_count > 1_000_000 {
        return Err(format!(
            "unreasonable secondary probe-network count {secondary_count}"
        ));
    }
    let mut secondary = Vec::with_capacity(secondary_count);
    for _ in 0..secondary_count {
        if version < 6 {
            cursor = cursor
                .checked_add(12)
                .ok_or("probe-network offset overflow")?;
        }
        let length = u32le(bytes, cursor)? as usize;
        cursor = cursor
            .checked_add(4)
            .ok_or("probe-network offset overflow")?;
        if length > 1 << 20 {
            return Err(format!(
                "unreasonable secondary resource name length {length}"
            ));
        }
        let end = cursor
            .checked_add(length)
            .ok_or("probe-network offset overflow")?;
        let raw = bytes
            .get(cursor..end)
            .ok_or("truncated secondary resource name")?;
        let name = std::str::from_utf8(raw).map_err(|error| error.to_string())?;
        secondary.push(name.trim_end_matches('\0').to_owned());
        cursor = end;
        if version < 6 {
            cursor = cursor
                .checked_add(4)
                .ok_or("probe-network offset overflow")?;
        }
    }
    Ok((version, names, secondary, cursor))
}

#[derive(Clone, Copy, Debug)]
pub struct Tetrahedron {
    pub vertex_indices: [u32; 4],
    pub probe_indices: [u16; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct TetrahedronVertex {
    pub position: [f32; 3],
    pub probe_weights: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub probe_name: String,
    pub first_tetrahedron: usize,
    pub tetrahedron_count: usize,
    pub planes: Vec<([f32; 3], [f32; 3], bool)>,
    pub occluder_samples: Vec<usize>,
}
#[derive(Clone, Debug)]
pub struct RuntimeTetrahedron {
    pub shader_index: usize,
    pub cell_index: u32,
    pub positions: [[f32; 3]; 4],
}
#[derive(Clone, Debug)]
pub struct SpatialData {
    pub cells: Vec<Cell>,
    pub runtime_tetrahedra: Vec<RuntimeTetrahedron>,
    pub voronoi_version: u32,
    pub cell_count: usize,
    pub tetrahedra: Vec<Tetrahedron>,
    pub vertices: Vec<TetrahedronVertex>,
    pub bytes_consumed: usize,
}

/// Read the CPU Voronoi query geometry and indirect-lighting shader buffers.
/// PNW v7 embeds Voronoi v9; cell/runtime layouts follow the native loaders,
/// while packed shader buffers follow tetrahedron_t/tetrahedronVertex_t.
pub fn spatial_data(bytes: &[u8], offset: usize) -> Result<SpatialData, String> {
    let mut cursor = offset;
    let voronoi_version = u32le(bytes, cursor)?;
    cursor += 4;
    if voronoi_version < 9 {
        return Err(format!(
            "unsupported Voronoi data version {voronoi_version}; reader is verified for v9"
        ));
    }
    // FUN_1403843f0 reads a feature byte for Voronoi versions above 2; this
    // gates optional fields in each cell neighbor record.
    let data_feature = *bytes.get(cursor).ok_or("truncated Voronoi feature byte")?;
    cursor += 1;
    let cell_count = u32le(bytes, cursor)? as usize;
    cursor += 4;
    if cell_count > 1_000_000 {
        return Err(format!("unreasonable Voronoi cell count {cell_count}"));
    }
    let mut cells = Vec::with_capacity(cell_count);
    for cell_index in 0..cell_count {
        let name_len = u32le(bytes, cursor)? as usize;
        cursor = cursor
            .checked_add(4 + name_len)
            .ok_or("Voronoi cell name offset overflow")?;
        if cursor > bytes.len() {
            return Err("truncated Voronoi cell name".into());
        }
        let name_start = cursor - name_len;
        let probe_name = std::str::from_utf8(&bytes[name_start..cursor])
            .map_err(|e| e.to_string())?
            .trim_end_matches('\0')
            .to_owned();
        let first_tetrahedron = u32le(bytes, cursor + 9)? as usize;
        let cell_tetrahedron_count = u32le(bytes, cursor + 13)? as usize;
        let mut planes = Vec::new();
        // FUN_140384870 fixed fields through the variable neighbor list.
        cursor = cursor
            .checked_add(29)
            .ok_or("Voronoi cell offset overflow")?;
        let neighbor_records = u32le(bytes, cursor - 4)? as usize;
        if neighbor_records > 1_000_000 {
            return Err(format!(
                "unreasonable Voronoi neighbor count {neighbor_records}"
            ));
        }
        for neighbor_index in 0..neighbor_records {
            let f = |at| u32le(bytes, cursor + at).map(f32::from_bits);
            let point = [f(8)?, f(12)?, f(16)?];
            let normal = [f(20)?, f(24)?, f(28)?];
            let skip = voronoi_version >= 7
                && *bytes.get(cursor + 45).ok_or("truncated neighbor flag")? != 0;
            planes.push((point, normal, skip));
            cursor = cursor
                .checked_add(
                    44 + usize::from(voronoi_version >= 2) + usize::from(voronoi_version >= 7),
                )
                .ok_or("Voronoi neighbor offset overflow")?;
            let vertex_count = u32le(bytes, cursor)? as usize;
            cursor += 4;
            if vertex_count > 1_000_000 {
                return Err(format!(
                    "unreasonable Voronoi neighbor vertex count {vertex_count} at cell {cell_index}, neighbor {neighbor_index}, offset {cursor:#x}, feature {data_feature}"
                ));
            }
            let per_vertex_extra =
                usize::from(voronoi_version < 6) * 4 + usize::from(voronoi_version < 4) * 4;
            cursor = cursor
                .checked_add(
                    vertex_count
                        .checked_mul(12 + per_vertex_extra)
                        .ok_or("Voronoi vertex count overflow")?,
                )
                .ok_or("Voronoi neighbor vertices offset overflow")?;
            if cursor > bytes.len() {
                return Err("truncated Voronoi neighbor vertices".into());
            }
        }
        let face_count = u32le(bytes, cursor)? as usize;
        cursor += 4;
        if face_count > 1_000_000 {
            return Err(format!("unreasonable Voronoi face count {face_count}"));
        }
        let occluder_samples = (0..face_count)
            .map(|i| u32le(bytes, cursor + i * 4).map(|v| v as usize))
            .collect::<Result<Vec<_>, _>>()?;
        cells.push(Cell {
            probe_name,
            first_tetrahedron,
            tetrahedron_count: cell_tetrahedron_count,
            planes,
            occluder_samples,
        });
        cursor = cursor
            .checked_add(
                face_count
                    .checked_mul(4)
                    .ok_or("Voronoi face count overflow")?,
            )
            .and_then(|value| value.checked_add(24))
            .ok_or("Voronoi cell tail offset overflow")?;
        if cursor > bytes.len() {
            return Err("truncated Voronoi cell tail".into());
        }
    }

    let tetrahedron_count = u32le(bytes, cursor)? as usize;
    cursor += 4;
    if tetrahedron_count > 10_000_000 {
        return Err(format!(
            "unreasonable Voronoi tetrahedron count {tetrahedron_count}"
        ));
    }
    // Keep the CPU tetrahedra and cell ranges: shader tetrahedra overlap and
    // cannot be searched globally to reproduce FUN_140385da0.
    let mut runtime_tetrahedra = Vec::with_capacity(tetrahedron_count);
    for _ in 0..tetrahedron_count {
        let shader_index = u32le(bytes, cursor)? as usize;
        let cell_index = u32le(bytes, cursor + 4)?;
        let mut positions = [[0.; 3]; 4];
        for i in 0..4 {
            for c in 0..3 {
                positions[i][c] = f32::from_bits(u32le(bytes, cursor + 8 + (i * 3 + c) * 4)?);
            }
        }
        runtime_tetrahedra.push(RuntimeTetrahedron {
            shader_index,
            cell_index,
            positions,
        });
        cursor += 56;
    }

    // FUN_1403843f0 reads raw byte sizes for 24-byte tetrahedron_t and
    // 16-byte tetrahedronVertex_t shader records.
    let tetrahedron_bytes = u32le(bytes, cursor)? as usize;
    cursor += 4;
    if tetrahedron_bytes % 24 != 0 {
        return Err(format!(
            "tetrahedron shader buffer has invalid byte size {tetrahedron_bytes}"
        ));
    }
    let tetrahedron_end = cursor
        .checked_add(tetrahedron_bytes)
        .ok_or("tetrahedron shader buffer offset overflow")?;
    let raw_tetrahedra = bytes
        .get(cursor..tetrahedron_end)
        .ok_or("truncated tetrahedron shader buffer")?;
    let mut tetrahedra = Vec::with_capacity(tetrahedron_bytes / 24);
    for record in raw_tetrahedra.chunks_exact(24) {
        let read_u32 = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        let first_probe_pair = read_u32(16);
        let second_probe_pair = read_u32(20);
        tetrahedra.push(Tetrahedron {
            vertex_indices: [read_u32(0), read_u32(4), read_u32(8), read_u32(12)],
            probe_indices: [
                first_probe_pair as u16,
                (first_probe_pair >> 16) as u16,
                second_probe_pair as u16,
                (second_probe_pair >> 16) as u16,
            ],
        });
    }
    cursor = tetrahedron_end;

    let vertex_bytes = u32le(bytes, cursor)? as usize;
    cursor += 4;
    if vertex_bytes % 16 != 0 {
        return Err(format!(
            "tetrahedron-vertex buffer has invalid byte size {vertex_bytes}"
        ));
    }
    let vertex_end = cursor
        .checked_add(vertex_bytes)
        .ok_or("tetrahedron-vertex buffer offset overflow")?;
    let raw_vertices = bytes
        .get(cursor..vertex_end)
        .ok_or("truncated tetrahedron-vertex shader buffer")?;
    let mut vertices = Vec::with_capacity(vertex_bytes / 16);
    for record in raw_vertices.chunks_exact(16) {
        let f32_at = |at: usize| f32::from_le_bytes(record[at..at + 4].try_into().unwrap());
        let packed = u32::from_le_bytes(record[12..16].try_into().unwrap());
        vertices.push(TetrahedronVertex {
            position: [f32_at(0), f32_at(4), f32_at(8)],
            probe_weights: [
                packed as u8,
                (packed >> 8) as u8,
                (packed >> 16) as u8,
                (packed >> 24) as u8,
            ],
        });
    }
    cursor = vertex_end;

    if tetrahedra.iter().any(|tetra| {
        tetra
            .vertex_indices
            .iter()
            .any(|&index| index as usize >= vertices.len())
    }) {
        return Err("tetrahedron references a vertex outside its shader buffer".into());
    }
    if cells.iter().any(|c| {
        c.first_tetrahedron
            .checked_add(c.tetrahedron_count)
            .is_none_or(|end| end > runtime_tetrahedra.len())
    }) || runtime_tetrahedra
        .iter()
        .any(|t| t.shader_index >= tetrahedra.len())
    {
        return Err("Voronoi CPU query references an invalid tetrahedron".into());
    }
    Ok(SpatialData {
        voronoi_version,
        cell_count,
        cells,
        runtime_tetrahedra,
        tetrahedra,
        vertices,
        bytes_consumed: cursor,
    })
}

/// PNW buffers bound by FUN_1402afc60 for the static world's vertex shader.
#[derive(Clone, Debug)]
pub struct VertexReferences {
    pub cell_probe_indices: Vec<[u32; 4]>,
    pub instance_offsets: Vec<u32>,
    pub tetrahedron_indices: Vec<u32>,
    pub bytes_consumed: usize,
}

/// Read the remainder after FUN_1403843f0's Voronoi/shader buffers:
/// four probe indices per cell, FUN_140290ab0's vertex-reference buffer,
/// one status byte, then FUN_140290c10's instance-offset buffer. Descriptor
/// names14229ad50/14229ad70 establish this order. The native readers
/// consume `(byte_count >> 2) * 4`, not the declared unrounded byte count.
pub fn vertex_references(
    bytes: &[u8],
    offset: usize,
    secondary_probe_count: usize,
) -> Result<VertexReferences, String> {
    let mut cursor = offset;
    let rows = u32le(bytes, cursor)? as usize;
    let columns = u32le(bytes, cursor + 4)? as usize;
    cursor += 8;
    if rows > 1_000_000 || columns > 4 {
        return Err(format!(
            "invalid probe-index table dimensions {rows}x{columns}"
        ));
    }
    let table_bytes = rows
        .checked_mul(columns)
        .and_then(|n| n.checked_mul(4))
        .ok_or("probe-index table size overflow")?;
    let table_end = cursor
        .checked_add(table_bytes)
        .ok_or("probe-index table offset overflow")?;
    let raw = bytes
        .get(cursor..table_end)
        .ok_or("truncated probe-index table")?;
    let mut cell_probe_indices = vec![[u32::MAX; 4]; rows];
    for (i, word) in raw.chunks_exact(4).enumerate() {
        let index = u32::from_le_bytes(word.try_into().unwrap());
        // FUN_1402b15e0 clamps non-sentinel out-of-range secondary IDs to 0.
        cell_probe_indices[i / columns][i % columns] = if secondary_probe_count != 0
            && index != u32::MAX
            && index as usize >= secondary_probe_count
        {
            0
        } else {
            index
        };
    }
    cursor = table_end;
    let tetrahedron_indices = read_word_buffer(bytes, &mut cursor)?;
    let status = *bytes
        .get(cursor)
        .ok_or("truncated probe-network buffer status")?;
    cursor += 1;
    if status != 0 {
        return Err("probe-network vertex lighting unavailable (native load status 9)".into());
    }
    let instance_offsets = read_word_buffer(bytes, &mut cursor)?;
    Ok(VertexReferences {
        cell_probe_indices,
        instance_offsets,
        tetrahedron_indices,
        bytes_consumed: cursor,
    })
}

fn read_word_buffer(bytes: &[u8], cursor: &mut usize) -> Result<Vec<u32>, String> {
    let size = (u32le(bytes, *cursor)? as usize >> 2) * 4;
    *cursor = cursor
        .checked_add(4)
        .ok_or("probe-network buffer offset overflow")?;
    let end = cursor
        .checked_add(size)
        .ok_or("probe-network buffer size overflow")?;
    let raw = bytes
        .get(*cursor..end)
        .ok_or("truncated probe-network word buffer")?;
    *cursor = end;
    Ok(raw
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_v7_ordered_names() {
        let bytes = b"PNW\x07\x01\0\0\0\x03\0\0\0foo\0\0\0\0";
        let (version, names, secondary, end) = ordered_probe_names(bytes).unwrap();
        assert_eq!(version, 7);
        assert_eq!(names, ["foo"]);
        assert!(secondary.is_empty());
        assert_eq!(end, bytes.len());
    }

    #[test]
    fn reads_native_vertex_reference_tail() {
        let mut bytes = Vec::new();
        for word in [1u32, 2, 7, 9, 8, 11, 22] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        for word in [12u32, 3, 4, 5] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let tail = vertex_references(&bytes, 0, 8).unwrap();
        assert_eq!(tail.cell_probe_indices, [[7, 0, u32::MAX, u32::MAX]]);
        assert_eq!(tail.tetrahedron_indices, [11, 22]);
        assert_eq!(tail.instance_offsets, [3, 4, 5]);
        assert_eq!(tail.bytes_consumed, bytes.len());
        bytes[28] = 1;
        assert!(vertex_references(&bytes, 0, 8)
            .unwrap_err()
            .contains("status 9"));
    }

    #[test]
    fn native_word_buffers_floor_the_declared_size() {
        let mut bytes = Vec::new();
        for word in [0u32, 4, 5, 123] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let tail = vertex_references(&bytes, 0, 0).unwrap();
        assert_eq!(tail.tetrahedron_indices, [123]);
        assert!(tail.instance_offsets.is_empty());
        assert_eq!(tail.bytes_consumed, bytes.len());
        assert!(vertex_references(&bytes[..bytes.len() - 1], 0, 0).is_err());
    }
}
