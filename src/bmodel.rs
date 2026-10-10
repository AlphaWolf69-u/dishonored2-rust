//! Read-only decoder for static BML1/BTRI model geometry.

#[derive(Debug)]
pub struct Model {
    pub surfaces: Vec<Surface>,
    /// FUN_140269ff0 unions the authored BTRI surface bounds.
    pub authored_bounds: [[f32; 3]; 2],
    pub material_bindings: Vec<String>,
    /// Bytes left after the material-binding table, not silently interpreted.
    pub bytes_after_bindings: usize,
}

#[derive(Debug)]
pub struct Surface {
    pub material_path: String,
    /// BML1 surface separator/marker preserved from the source record.
    pub surface_marker: u32,
    /// FUN_14025a040 reads six finite floats after the index stream.
    pub authored_bounds: [[f32; 3]; 2],
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Declaration 1: UV1 half2, normal/tangent SNORM8x4, color UNORM8x4.
    pub packed_channels: Vec<[u8; 16]>,
    pub indices: Vec<u16>,
}

fn region(bytes: &[u8], at: usize, size: usize) -> Result<&[u8], String> {
    bytes
        .get(at..at.checked_add(size).ok_or("offset overflow")?)
        .ok_or_else(|| format!("model section at {at} exceeds {} bytes", bytes.len()))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        region(bytes, at, 4)?.try_into().unwrap(),
    ))
}

fn float_at(bytes: &[u8], at: usize) -> Result<f32, String> {
    let value = f32::from_le_bytes(region(bytes, at, 4)?.try_into().unwrap());
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("non-finite model coordinate at {at}"))
    }
}

/// FUN_140269ff0: union authored, non-null surface bounds into a resource.
/// Empty lists retain prior bounds. Null geometry/shader recovery belongs to
/// the owning loader and is not represented by this typed helper.
pub fn accumulate_resource_bounds(previous: [[f32; 3]; 2], surfaces: &[[[f32; 3]; 2]]) -> [[f32; 3]; 2] {
    if surfaces.is_empty() { return previous; }
    let initial = f32::from_bits(0x7149_f2ca);
    let mut result = [[initial; 3], [-initial; 3]];
    for surface in surfaces {
        for axis in 0..3 {
            if surface[0][axis] <= result[0][axis] { result[0][axis] = surface[0][axis]; }
            if result[1][axis] <= surface[1][axis] { result[1][axis] = surface[1][axis]; }
        }
    }
    if result[1][0] <= result[0][0] && result[0][0] != result[1][0] { [[0.0; 3]; 2] } else { result }
}

pub fn parse(bytes: &[u8]) -> Result<Model, String> {
    if region(bytes, 0, 4)? != b"1LMB" {
        return Err("not a BML1 model".into());
    }
    let source_length = u32_at(bytes, 8)? as usize;
    if source_length > 4096 {
        return Err("unreasonable BML1 source path length".into());
    }
    region(bytes, 12, source_length)?;
    let surface_count = u32_at(bytes, 12 + source_length)? as usize;
    if surface_count == 0 || surface_count > 128 {
        return Err("unreasonable BML1 surface count".into());
    }
    let mut at = 16 + source_length;
    let mut surfaces = Vec::with_capacity(surface_count);
    for surface_number in 0..surface_count {
        let length = u32_at(bytes, at)? as usize;
        at += 4;
        if length == 0 || length > 4096 {
            return Err(format!("surface {surface_number} material length invalid"));
        }
        let material_path = std::str::from_utf8(region(bytes, at, length)?)
            .map_err(|error| error.to_string())?
            .to_string();
        at += length;
        let surface_marker = u32_at(bytes, at)?;
        // Ordinary BML1 surfaces use 0xffffffff. The installed area-volume
        // proxy meshes use zero at the same position and still contain valid
        // BTRI geometry immediately afterward.
        if surface_marker != u32::MAX && surface_marker != 0 {
            return Err(format!(
                "surface {surface_number} has unsupported marker {surface_marker:#x}"
            ));
        }
        at += 4;
        if region(bytes, at, 4)? != b"BTRI" {
            return Err(format!("surface {surface_number} missing BTRI"));
        }
        let btri = at;
        if u32_at(bytes, btri + 4)? != 0x0001_0002 {
            return Err(format!(
                "surface {surface_number} has unsupported BTRI version"
            ));
        }
        let cpu_declaration = u32_at(bytes, btri + 8)?;
        let gpu_declaration = u32_at(bytes, btri + 12)?;
        if cpu_declaration != 1 || gpu_declaration != 1 || region(bytes, btri + 0x3c, 8)? != [0; 8] {
            return Err(format!("surface {surface_number} unsupported vertex declarations {cpu_declaration}/{gpu_declaration} or GPU stream"));
        }
        let vertex_count = u32_at(bytes, btri + 0x38)? as usize;
        if vertex_count == 0 || vertex_count > u16::MAX as usize {
            return Err(format!("surface {surface_number} vertex count invalid"));
        }
        let vertex_start = btri + 0x44;
        let vertex_bytes = region(
            bytes,
            vertex_start,
            vertex_count
                .checked_mul(32)
                .ok_or("vertex count overflow")?,
        )?;
        let mut positions = Vec::with_capacity(vertex_count);
        let mut uvs = Vec::with_capacity(vertex_count);
        let mut packed_channels = Vec::with_capacity(vertex_count);
        for vertex in vertex_bytes.chunks_exact(32) {
            let position = [
                float_at(vertex, 0)?,
                float_at(vertex, 4)?,
                float_at(vertex, 8)?,
            ];
            let u = half::f16::from_bits(u16::from_le_bytes(vertex[12..14].try_into().unwrap()))
                .to_f32();
            let v = half::f16::from_bits(u16::from_le_bytes(vertex[14..16].try_into().unwrap()))
                .to_f32();
            if !u.is_finite() || !v.is_finite() {
                return Err(format!("surface {surface_number} has invalid UVs"));
            }
            positions.push(position);
            uvs.push([u, v]);
            packed_channels.push(vertex[16..32].try_into().unwrap());
        }
        at = vertex_start + vertex_bytes.len();
        let index_count = u32_at(bytes, at)? as usize;
        at += 4;
        if index_count == 0 || index_count % 3 != 0 || index_count > 3_000_000 {
            return Err(format!("surface {surface_number} index count invalid"));
        }
        let index_bytes = region(
            bytes,
            at,
            index_count.checked_mul(2).ok_or("index count overflow")?,
        )?;
        let mut indices = Vec::with_capacity(index_count);
        for pair in index_bytes.chunks_exact(2) {
            let index = u16::from_le_bytes(pair.try_into().unwrap());
            if index as usize >= vertex_count {
                return Err(format!(
                    "surface {surface_number} index {index} exceeds {vertex_count} vertices"
                ));
            }
            indices.push(index);
        }
        at += index_bytes.len();
        // FUN_14025a040: authored minimum/maximum then the seventh field.
        let authored_bounds = [
            [float_at(bytes, at)?, float_at(bytes, at + 4)?, float_at(bytes, at + 8)?],
            [float_at(bytes, at + 12)?, float_at(bytes, at + 16)?, float_at(bytes, at + 20)?],
        ];
        float_at(bytes, at + 24)?;
        at += 28;
        surfaces.push(Surface {
            material_path,
            surface_marker,
            authored_bounds,
            positions,
            uvs,
            packed_channels,
            indices,
        });
        if region(bytes, at, 4)? != b"BML1" {
            return Err(format!(
                "surface {surface_number} BML1 section boundary missing"
            ));
        }
        at += 4;
    }
    let binding_count = u32_at(bytes, at)? as usize;
    at += 4;
    if binding_count > 128 {
        return Err("unreasonable BML1 material-binding count".into());
    }
    let mut material_bindings = Vec::with_capacity(binding_count);
    for _ in 0..binding_count {
        let length = u32_at(bytes, at)? as usize;
        at += 4;
        if length > 4096 {
            return Err("unreasonable BML1 material-binding path length".into());
        }
        let path = std::str::from_utf8(region(bytes, at, length)?)
            .map_err(|error| error.to_string())?
            .to_string();
        at += length;
        region(bytes, at, 12)?;
        at += 12;
        material_bindings.push(path);
    }
    let surface_bounds: Vec<_> = surfaces.iter().map(|surface| surface.authored_bounds).collect();
    let authored_bounds = accumulate_resource_bounds([[0.0; 3]; 2], &surface_bounds);
    Ok(Model {
        surfaces,
        authored_bounds,
        material_bindings,
        bytes_after_bindings: bytes.len() - at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retains_authored_bounds_instead_of_vertex_extrema() {
        let mut bytes = Vec::new();
        let u32_bytes = |bytes: &mut Vec<u8>, value: u32| bytes.extend(value.to_le_bytes());
        bytes.extend(b"1LMB");
        u32_bytes(&mut bytes, 0);
        u32_bytes(&mut bytes, 3);
        bytes.extend(b"x.y");
        u32_bytes(&mut bytes, 1);
        u32_bytes(&mut bytes, 1);
        bytes.extend(b"m");
        u32_bytes(&mut bytes, u32::MAX);
        let mut header = [0u8; 0x44];
        header[..4].copy_from_slice(b"BTRI");
        header[4..8].copy_from_slice(&0x0001_0002u32.to_le_bytes());
        header[8..12].copy_from_slice(&1u32.to_le_bytes());
        header[12..16].copy_from_slice(&1u32.to_le_bytes());
        header[0x38..0x3c].copy_from_slice(&3u32.to_le_bytes());
        bytes.extend(header);
        for point in [[0.0f32; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for value in point { bytes.extend(value.to_le_bytes()); }
            bytes.extend([0; 20]);
        }
        u32_bytes(&mut bytes, 3);
        for index in [0u16, 1, 2] { bytes.extend(index.to_le_bytes()); }
        let bounds = [[-10.0f32, -20.0, -30.0], [40.0, 50.0, 60.0]];
        for side in bounds { for value in side { bytes.extend(value.to_le_bytes()); } }
        bytes.extend(7.0f32.to_le_bytes());
        bytes.extend(b"BML1");
        u32_bytes(&mut bytes, 0);
        let model = parse(&bytes).unwrap();
        assert_eq!(model.surfaces[0].authored_bounds, bounds);
        assert_eq!(model.authored_bounds, bounds);
    }

    #[test]
    fn rejects_truncated_model() {
        assert!(parse(b"BML1").is_err());
    }
}
