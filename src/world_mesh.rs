//! Static-world BWM geometry. Other BWM sections (materials and instances) follow later.

/// FUN_1402d1a00: first semantic/index match; missing uses UINT32_MAX.
/// A validated slice represents the native packed low-24-bit element count.
pub fn find_vertex_element(elements: &[[u8; 8]], semantic: u8, index: u8) -> u32 {
    elements.iter().position(|e| e[4] == semantic && e[5] == index)
        .map_or(u32::MAX, |i| i as u32)
}

// Live declaration 8, captured from initialized DAT_1432d55a0. Stream 0
// contains positions; stream 1 contains both UVs, N/T and color (20 bytes).
const BWM_ELEMENTS: [[u8; 8]; 6] = [
    [0, 1, 7, 9, 0, 0, 0, 0],
    [0, 1, 7, 5, 4, 0, 0, 1],
    [0, 1, 7, 5, 4, 1, 4, 1],
    [0, 1, 4, 10, 1, 0, 8, 1],
    [0, 1, 4, 10, 2, 0, 12, 1],
    [0, 1, 5, 10, 5, 0, 16, 1],
];

#[derive(Debug)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Declaration 8 stream 1: UV1 half2, normal/tangent SNORM8x4, color UNORM8x4.
    pub packed_channels: Vec<[u8; 16]>,
    pub indices: Vec<u16>,
}

#[derive(Debug)]
pub struct WorldMesh {
    pub meshes: Vec<Mesh>,
    /// Row-major 3x4 affine transforms in game coordinates (Z up).
    pub matrices: Vec<[f32; 12]>,
    /// BWM v21 bounds and Umbra IDs, indexed alongside `matrices`.
    pub visibility: Vec<WorldVisibility>,
    /// Four BWM instance lists handled by the original renderer after Umbra.
    pub auxiliary_instance_groups: [Vec<usize>; 4],
    /// A visible parent suppresses these child BWM instances after the query.
    pub visibility_groups: Vec<WorldVisibilityGroup>,
    /// BWM v21 render-instance indices for each Tome object index.
    pub tome_object_instances: Vec<Vec<usize>>,
    /// Separate world-record list passed to the original spatial query path.
    pub spatial_query_instances: Vec<usize>,
    pub objects: Vec<WorldObject>,
}

#[derive(Debug)]
pub struct WorldVisibility {
    pub bounds: [[f32; 3]; 2],
    pub umbra_id: u32,
    pub trailing_field: u16,
}

#[derive(Debug)]
pub struct WorldVisibilityGroup {
    pub parent: usize,
    pub children: Vec<usize>,
}

#[derive(Debug)]
pub struct WorldObject {
    pub mesh_index: usize,
    pub material_path: String,
    pub matrix_start: usize,
    pub matrix_end: usize,
    pub flipped: bool,
    pub lod: usize,
}

fn region(bytes: &[u8], start: usize, size: usize) -> Result<&[u8], String> {
    bytes
        .get(start..start.checked_add(size).ok_or("offset overflow")?)
        .ok_or_else(|| format!("section at {start} of {size} bytes exceeds file"))
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    let data: [u8; 2] = region(bytes, at, 2)?.try_into().unwrap();
    Ok(u16::from_le_bytes(data))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    let data: [u8; 4] = region(bytes, at, 4)?.try_into().unwrap();
    Ok(u32::from_le_bytes(data))
}

fn usize_at(bytes: &[u8], at: usize) -> Result<usize, String> {
    usize::try_from(u32_at(bytes, at)?).map_err(|_| "size does not fit usize".into())
}

fn counted_indices(bytes: &[u8], at: &mut usize, limit: usize) -> Result<Vec<usize>, String> {
    let count = usize_at(bytes, *at)?;
    *at = at.checked_add(4).ok_or("offset overflow")?;
    let data = region(
        bytes,
        *at,
        count.checked_mul(4).ok_or("index size overflow")?,
    )?;
    let mut indices = Vec::with_capacity(count);
    for word in data.chunks_exact(4) {
        let index = u32::from_le_bytes(word.try_into().unwrap()) as usize;
        if index >= limit {
            return Err(format!("world visibility index {index} exceeds {limit}"));
        }
        indices.push(index);
    }
    *at += data.len();
    Ok(indices)
}

fn next_block<'a>(bytes: &'a [u8], at: &mut usize) -> Result<&'a [u8], String> {
    let length = usize_at(bytes, *at)?;
    let begin = at.checked_add(4).ok_or("offset overflow")?;
    let block = region(bytes, begin, length)?;
    *at = begin + length;
    Ok(block)
}

pub fn parse(bytes: &[u8]) -> Result<WorldMesh, String> {
    if region(bytes, 0, 4)? != b"BWM1" {
        return Err("not a BWM1 file".into());
    }
    if u16_at(bytes, 4)? != 21 || u16_at(bytes, 6)? != 1 || u32_at(bytes, 8)? != 2 {
        return Err("unsupported BWM version".into());
    }
    let mut at = 12;
    let positions = next_block(bytes, &mut at)?;
    let attributes = next_block(bytes, &mut at)?;
    let indices = next_block(bytes, &mut at)?;
    let matrices = next_block(bytes, &mut at)?;
    let _unknown = next_block(bytes, &mut at)?;
    if positions.len() % 12 != 0
        || attributes.len() % 20 != 0
        || indices.len() % 2 != 0
        || matrices.len() % 48 != 0
    {
        return Err("misaligned world geometry section".into());
    }
    let mut decoded_matrices = Vec::with_capacity(matrices.len() / 48);
    for record in matrices.chunks_exact(48) {
        let mut transform = [0.0; 12];
        for (index, value) in transform.iter_mut().enumerate() {
            *value = f32::from_le_bytes(record[index * 4..index * 4 + 4].try_into().unwrap());
            if !value.is_finite() {
                return Err("non-finite world transform".into());
            }
        }
        decoded_matrices.push(transform);
    }
    let count = usize_at(bytes, at)?;
    if count > 100_000 {
        return Err("unreasonable mesh count".into());
    }
    at += 4;
    let mut meshes = Vec::with_capacity(count);
    for mesh_number in 0..count {
        let record = region(bytes, at, 41)?;
        at += 41;
        let u32_field = |offset| usize_at(record, offset);
        if u32_field(0)? != 2
            || u32_field(8)? != 12
            || u32_field(16)? != 20
            || u32_field(20)? != 0x1234_5678
            || u32_field(36)? != 3
            || record[40] != 1
        {
            return Err(format!("mesh {mesh_number} has an unsupported layout"));
        }
        let vertex_offset = u32_field(4)?;
        let attr_offset = u32_field(12)?;
        let index_offset = u32_field(24)?;
        let index_count = u32_field(28)?;
        let vertex_count = u32_field(32)?;
        if index_count % 3 != 0 {
            return Err(format!("mesh {mesh_number} has incomplete triangles"));
        }
        let coords = region(
            positions,
            vertex_offset,
            vertex_count
                .checked_mul(12)
                .ok_or("vertex count overflow")?,
        )?;
        let vertex_attributes = region(
            attributes,
            attr_offset,
            vertex_count
                .checked_mul(20)
                .ok_or("attribute count overflow")?,
        )?;
        let faces = region(
            indices,
            index_offset,
            index_count.checked_mul(2).ok_or("index count overflow")?,
        )?;
        let mut mesh_positions = Vec::with_capacity(vertex_count);
        let mut mesh_uvs = Vec::with_capacity(vertex_count);
        let mut packed_channels = Vec::with_capacity(vertex_count);
        for vertex in coords.chunks_exact(12) {
            let mut position = [0.0; 3];
            for (axis, value) in position.iter_mut().enumerate() {
                *value = f32::from_le_bytes(vertex[axis * 4..axis * 4 + 4].try_into().unwrap());
                if !value.is_finite() {
                    return Err(format!(
                        "mesh {mesh_number} contains non-finite coordinates"
                    ));
                }
            }
            mesh_positions.push(position);
        }
        let channel_offsets = [(4, 1), (1, 0), (2, 0), (5, 0)].map(|(semantic, index)| {
            BWM_ELEMENTS[find_vertex_element(&BWM_ELEMENTS, semantic, index) as usize][6] as usize
        });
        for vertex in vertex_attributes.chunks_exact(20) {
            let u =
                half::f16::from_bits(u16::from_le_bytes(vertex[0..2].try_into().unwrap())).to_f32();
            let v =
                half::f16::from_bits(u16::from_le_bytes(vertex[2..4].try_into().unwrap())).to_f32();
            if !u.is_finite() || !v.is_finite() {
                return Err(format!("mesh {mesh_number} has non-finite UVs"));
            }
            mesh_uvs.push([u, v]);
            let mut packed = [0; 16];
            for (slot, offset) in channel_offsets.into_iter().enumerate() {
                packed[slot * 4..slot * 4 + 4].copy_from_slice(&vertex[offset..offset + 4]);
            }
            packed_channels.push(packed);
        }
        let mut mesh_indices = Vec::with_capacity(index_count);
        for pair in faces.chunks_exact(2) {
            let index = u16::from_le_bytes(pair.try_into().unwrap());
            if index as usize >= vertex_count {
                return Err(format!(
                    "mesh {mesh_number} references vertex {index} of {vertex_count}"
                ));
            }
            mesh_indices.push(index);
        }
        meshes.push(Mesh {
            positions: mesh_positions,
            uvs: mesh_uvs,
            packed_channels,
            indices: mesh_indices,
        });
    }
    let object_count = usize_at(bytes, at)?;
    if object_count > 100_000 {
        return Err("unreasonable object count".into());
    }
    at += 4;
    let mut objects = Vec::with_capacity(object_count);
    for object_number in 0..object_count {
        let matrix_start = usize_at(bytes, at)?;
        let matrix_end = usize_at(bytes, at + 4)?;
        let mesh_index = usize_at(bytes, at + 8)?;
        if matrix_start > matrix_end
            || matrix_end > matrices.len() / 48
            || mesh_index >= meshes.len()
        {
            return Err(format!(
                "object {object_number} references missing matrix or mesh"
            ));
        }
        let flipped = *region(bytes, at + 12, 1)?.first().unwrap();
        if flipped > 1 {
            return Err(format!("object {object_number} has invalid flip flag"));
        }
        let path_length = usize_at(bytes, at + 13)?;
        if path_length > 4096 {
            return Err(format!(
                "object {object_number} has unreasonable material path"
            ));
        }
        let material_path = std::str::from_utf8(region(bytes, at + 17, path_length)?)
            .map_err(|_| format!("object {object_number} material path is not UTF-8"))?
            .to_string();
        let lod = usize_at(bytes, at + 17 + path_length)?;
        if lod > 100 {
            return Err(format!("object {object_number} has unreasonable LOD"));
        }
        at += 21 + path_length;
        objects.push(WorldObject {
            mesh_index,
            material_path,
            matrix_start,
            matrix_end,
            flipped: flipped != 0,
            lod,
        });
    }
    // FUN_140296240 reads this BWM v21 section into the world's +0x390
    // array: 24 bytes of bounds, a 32-bit Umbra ID, and a 16-bit field.
    let visibility_count = usize_at(bytes, at)?;
    at += 4;
    if visibility_count != decoded_matrices.len() {
        return Err(format!(
            "{visibility_count} visibility records for {} transforms",
            decoded_matrices.len()
        ));
    }
    let records = region(
        bytes,
        at,
        visibility_count
            .checked_mul(30)
            .ok_or("visibility size overflow")?,
    )?;
    let mut visibility = Vec::with_capacity(visibility_count);
    for (number, record) in records.chunks_exact(30).enumerate() {
        let mut bounds = [[0.0; 3]; 2];
        for (index, value) in bounds.iter_mut().flatten().enumerate() {
            *value = f32::from_le_bytes(record[index * 4..index * 4 + 4].try_into().unwrap());
            if !value.is_finite() {
                return Err(format!("visibility record {number} has non-finite bounds"));
            }
        }
        let umbra_id = u32::from_le_bytes(record[24..28].try_into().unwrap());
        let trailing_field = u16::from_le_bytes(record[28..30].try_into().unwrap());
        visibility.push(WorldVisibility {
            bounds,
            umbra_id,
            trailing_field,
        });
    }
    at += records.len();
    // FUN_140296240 then reads four auxiliary lists, variable-length group
    // records, Tome-indexed instance lists at world +0x408, and a separate
    // world-record list at +0x418.
    let aux_counts = (0..4)
        .map(|offset| usize_at(bytes, at + offset * 4))
        .collect::<Result<Vec<_>, _>>()?;
    at += 16;
    let mut auxiliary_instance_groups = std::array::from_fn(|_| Vec::new());
    for (group, count) in aux_counts.into_iter().enumerate() {
        let size = count.checked_mul(4).ok_or("auxiliary list size overflow")?;
        for word in region(bytes, at, size)?.chunks_exact(4) {
            let index = u32::from_le_bytes(word.try_into().unwrap()) as usize;
            if index >= visibility_count {
                return Err(format!(
                    "auxiliary visibility index {index} exceeds {visibility_count}"
                ));
            }
            auxiliary_instance_groups[group].push(index);
        }
        at += size;
    }
    let grouped_count = usize_at(bytes, at)?;
    at += 4;
    let mut visibility_groups = Vec::with_capacity(grouped_count);
    for _ in 0..grouped_count {
        let parent = usize_at(bytes, at)?;
        if parent >= visibility_count {
            return Err(format!(
                "visibility group parent {parent} exceeds {visibility_count}"
            ));
        }
        let count = usize_at(bytes, at + 4)?;
        at += 8;
        let size = count.checked_mul(4).ok_or("grouped list size overflow")?;
        let mut children = Vec::with_capacity(count);
        for word in region(bytes, at, size)?.chunks_exact(4) {
            let index = u32::from_le_bytes(word.try_into().unwrap()) as usize;
            if index >= visibility_count {
                return Err(format!(
                    "visibility group child {index} exceeds {visibility_count}"
                ));
            }
            children.push(index);
        }
        visibility_groups.push(WorldVisibilityGroup { parent, children });
        at += size;
    }
    let tome_count = usize_at(bytes, at)?;
    at += 4;
    if tome_count > bytes.len().saturating_sub(at) / 4 {
        return Err("Tome mapping count exceeds BWM length".into());
    }
    let mut tome_object_instances = Vec::with_capacity(tome_count);
    for _ in 0..tome_count {
        tome_object_instances.push(counted_indices(bytes, &mut at, visibility_count)?);
    }
    let spatial_query_instances = counted_indices(bytes, &mut at, visibility_count)?;
    if at != bytes.len() {
        return Err(format!("{} unparsed BWM bytes", bytes.len() - at));
    }
    Ok(WorldMesh {
        meshes,
        matrices: decoded_matrices,
        visibility,
        tome_object_instances,
        auxiliary_instance_groups,
        visibility_groups,
        spatial_query_instances,
        objects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_element_lookup_preserves_first_match_and_missing_sentinel() {
        assert_eq!(find_vertex_element(&BWM_ELEMENTS, 4, 1), 2);
        assert_eq!(find_vertex_element(&BWM_ELEMENTS, 5, 0), 5);
        assert_eq!(find_vertex_element(&BWM_ELEMENTS, 4, 3), u32::MAX);
        assert_eq!(find_vertex_element(&[], 0, 0), u32::MAX);
        assert_eq!(find_vertex_element(&[BWM_ELEMENTS[2]; 2], 4, 1), 0);
    }

    #[test]
    #[ignore = "requires local initialized-game registry capture"]
    fn vertex_element_lookup_matches_all_native_live_declarations() {
        let text = std::fs::read_to_string("artifacts/runtime-vertex-declarations.txt").unwrap();
        let mut verified = 0;
        for line in text.lines().filter(|l| l.starts_with("decl_raw ")) {
            let hex = line.split_whitespace().last().unwrap();
            let bytes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect::<Vec<_>>();
            let elements = bytes.chunks_exact(8).map(|e| e.try_into().unwrap()).collect::<Vec<[u8; 8]>>();
            for (index, element) in elements.iter().enumerate() {
                assert_eq!(find_vertex_element(&elements, element[4], element[5]), index as u32);
                verified += 1;
            }
            assert_eq!(find_vertex_element(&elements, 255, 255), u32::MAX);
        }
        assert_eq!(verified, 35);
    }

    #[test]
    fn rejects_truncated_file() {
        assert!(parse(b"BWM1").is_err());
    }
    #[test]
    fn rejects_wrong_signature() {
        assert!(parse(&[0; 12]).is_err());
    }

    #[test]
    fn decodes_v21_umbra_id_for_transform_index() {
        let mut bytes = b"BWM1".to_vec();
        bytes.extend_from_slice(&21u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        for size in [0u32, 0, 0, 48, 0] {
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.resize(bytes.len() + size as usize, 0);
        }
        bytes.extend_from_slice(&0u32.to_le_bytes()); // meshes
        bytes.extend_from_slice(&0u32.to_le_bytes()); // objects
        bytes.extend_from_slice(&1u32.to_le_bytes()); // visibility records
        for value in [-2.0f32, -1.0, 0.0, 2.0, 3.0, 4.0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0x1234_5678u32.to_le_bytes());
        bytes.extend_from_slice(&0xffffu16.to_le_bytes());
        for count in [1u32, 0, 0, 0] {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes()); // first auxiliary instance index
        bytes.extend_from_slice(&1u32.to_le_bytes()); // grouped records
        bytes.extend_from_slice(&0u32.to_le_bytes()); // parent index
        bytes.extend_from_slice(&1u32.to_le_bytes()); // child count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // child index
        bytes.extend_from_slice(&1u32.to_le_bytes()); // Tome object count
        bytes.extend_from_slice(&1u32.to_le_bytes()); // instance count
        bytes.extend_from_slice(&0u32.to_le_bytes()); // instance index
        bytes.extend_from_slice(&0u32.to_le_bytes()); // spatial-query list
        let world = parse(&bytes).unwrap();
        assert_eq!(
            world.visibility[0].bounds,
            [[-2.0, -1.0, 0.0], [2.0, 3.0, 4.0]]
        );
        assert_eq!(world.visibility[0].umbra_id, 0x1234_5678);
        assert_eq!(world.visibility[0].trailing_field, 0xffff);
        assert_eq!(world.tome_object_instances, vec![vec![0]]);
        assert_eq!(
            world.auxiliary_instance_groups,
            [vec![0], vec![], vec![], vec![]]
        );
        assert_eq!(world.visibility_groups.len(), 1);
        assert_eq!(world.visibility_groups[0].parent, 0);
        assert_eq!(world.visibility_groups[0].children, [0]);
        assert!(world.spatial_query_instances.is_empty());
    }
}
