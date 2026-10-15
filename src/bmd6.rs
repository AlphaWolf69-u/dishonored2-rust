//! Structural reader for compiled MD6 models (`MM6B`). Decodes the unskinned
//! CPU vertex streams; joint transforms and animation are not yet supported.
//! Field order follows the installed game's idMD6Model loader and BTRI v2
//! reader (Ghidra functions 0x14025aa00, 0x140259960, 0x14025a040).

#[derive(Debug)]
pub struct ModelInfo {
    pub surfaces: Vec<SurfaceInfo>,
    pub bytes_after_surfaces: usize,
    pub has_skeleton: bool,
    pub has_decoded_geometry: bool,
    pub authored_bounds: [[f32; 3]; 2],
}

#[derive(Debug)]
pub struct SurfaceInfo {
    pub name: String,
    pub material_path: String,
    pub cpu_vertex_declaration: u32,
    pub gpu_vertex_declaration: u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub attachment_count: u32,
    pub positions: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// UV1 half2, normal/tangent SNORM8x4; declarations 9/10 have no COLOR0.
    pub packed_channels: Vec<[u8; 12]>,
    pub skin_influences: u8,
    /// Retain original UNORM8 weights and UINT8 indices; no skinning guess.
    pub skin_weights: Vec<[u8; 8]>,
    pub skin_indices: Vec<[u8; 8]>,
    pub indices: Vec<u16>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

/// Resolve the LOD0 mesh named by a Void `.md6` declaration. The entity's
/// declaration name and compiled mesh name are not always the same.
pub fn compiled_mesh_path(declaration: &str) -> Option<String> {
    let lod = declaration
        .split_once("lod 0")?
        .1
        .split_once('{')?
        .1
        .split_once('}')?
        .0;
    let after = lod.split_once("mesh \"")?.1;
    let path = after.split_once('"')?.0;
    let stem = path.strip_suffix(".md6mesh")?;
    Some(format!("generated/basemodel/{stem}.bmd6model"))
}

pub fn resolve_compiled_mesh(
    compiled_guess: &str,
    mut read_declaration: impl FnMut(&str) -> Option<String>,
) -> Result<String, String> {
    let mut declaration =
        declaration_path(compiled_guess).ok_or("not a compiled MD6 model path")?;
    let mut visited = std::collections::HashSet::new();
    for _ in 0..16 {
        if !visited.insert(declaration.clone()) {
            return Err("MD6 inheritance cycle".into());
        }
        let Some(text) = read_declaration(&declaration) else {
            return Ok(compiled_guess.into());
        };
        if let Some(mesh) = compiled_mesh_path(&text) {
            return Ok(mesh);
        }
        let Some(inherited) = text
            .split_once("inherit \"")
            .and_then(|(_, rest)| rest.split_once('"').map(|(path, _)| path))
        else {
            return Ok(compiled_guess.into());
        };
        let stem = inherited
            .strip_suffix(".md6")
            .ok_or("invalid MD6 inherit path")?;
        declaration = format!("generated/decls/md6def/{stem}.md6.decl");
    }
    Err("MD6 inheritance exceeds 16 declarations".into())
}

pub fn declaration_path(compiled_guess: &str) -> Option<String> {
    let stem = compiled_guess
        .strip_prefix("generated/basemodel/")?
        .strip_suffix(".bmd6model")?;
    Some(format!("generated/decls/md6def/{stem}.md6.decl"))
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(count).ok_or("MD6 offset overflow")?;
        let result = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| format!("MD6 data truncated at {} (need {count} bytes)", self.at))?;
        self.at = end;
        Ok(result)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        if length > 4096 {
            return Err(format!("MD6 string too long: {length}"));
        }
        let raw = self.take(length)?;
        Ok(std::str::from_utf8(raw)
            .map_err(|error| error.to_string())?
            .trim_end_matches('\0')
            .to_string())
    }
}

pub fn inspect(bytes: &[u8]) -> Result<ModelInfo, String> {
    let mut input = Reader { bytes, at: 0 };
    if input.take(4)? != b"MM6B" {
        return Err("not an MM6B model".into());
    }
    let source_name = input.string()?;
    // The original loader reads a 16-bit skeleton reference when a source
    // name is present; models without one proceed directly to bounds.
    if !source_name.is_empty() {
        input.take(2)?;
    }
    let bounds_raw = input.take(24)?;
    let authored_bounds = [
        std::array::from_fn(|axis| {
            f32::from_le_bytes(bounds_raw[axis * 4..axis * 4 + 4].try_into().unwrap())
        }),
        std::array::from_fn(|axis| {
            f32::from_le_bytes(bounds_raw[12 + axis * 4..16 + axis * 4].try_into().unwrap())
        }),
    ];
    let surface_count = input.u32()? as usize;
    if surface_count > 256 {
        return Err(format!("invalid MD6 surface count: {surface_count}"));
    }
    let mut surfaces = Vec::with_capacity(surface_count);
    for number in 0..surface_count {
        let name = input.string()?;
        let material_path = input.string()?;
        input.take(4)?; // per-surface field at +0x30
        input.u8()?; // per-surface flag at +0x34
        input.take(4)?; // per-surface field at +0x38
        if input.take(4)? != b"BTRI" {
            return Err(format!("surface {number} missing BTRI"));
        }
        let version = input.u32()?;
        if version <= 0x0001_0001 {
            return Err(format!(
                "surface {number} uses unsupported BTRI version {version:#x}"
            ));
        }
        let cpu_vertex_declaration = input.u32()?;
        let gpu_vertex_declaration = input.u32()?;
        input.take(40)?; // BTRI v2 bounds and flags
        let vertex_count = input.u32()?;
        let gpu_buffer_size = input.u64()?;
        if vertex_count > 1_000_000 {
            return Err(format!("surface {number} vertex count too large"));
        }
        if gpu_buffer_size != 0 {
            return Err(format!(
                "surface {number} GPU stream layout not yet decoded"
            ));
        }
        // Byte counts are selected by CPU vertex declaration, as in the game
        // loader. These strides are validated against the following index and
        // per-surface records across the installed asset catalog.
        let stride = match cpu_vertex_declaration {
            9 => 36usize,
            10 => 44usize,
            other => {
                let candidates: Vec<_> = (24usize..=96)
                    .step_by(4)
                    .filter_map(|stride| {
                        let at = input
                            .at
                            .checked_add((vertex_count as usize).checked_mul(stride)?)?;
                        let raw = input.bytes.get(at..at + 4)?;
                        let count = u32::from_le_bytes(raw.try_into().ok()?);
                        (count <= 3_000_000 && count % 3 == 0).then_some((stride, count))
                    })
                    .collect();
                return Err(format!("surface {number} CPU vertex declaration {other} not yet decoded; index-count candidates={candidates:?}"));
            }
        };
        let vertex_bytes = input.take(
            (vertex_count as usize)
                .checked_mul(stride)
                .ok_or("MD6 vertex size overflow")?,
        )?;
        // The first CPU stream is a tightly packed XYZ position array. The
        // remaining declaration-specific attributes follow it in this file.
        let primary_stride = 12;
        let primary_len = vertex_count as usize * primary_stride;
        let (primary, secondary) = vertex_bytes.split_at(primary_len);
        let texcoord_stream = secondary
            .get(..(vertex_count as usize * 8))
            .ok_or("MD6 texture-coordinate stream truncated")?;
        let mut positions = Vec::with_capacity(vertex_count as usize);
        let mut uvs = Vec::with_capacity(vertex_count as usize);
        let mut packed_channels = Vec::with_capacity(vertex_count as usize);
        let mut skin_weights = Vec::with_capacity(vertex_count as usize);
        let mut skin_indices = Vec::with_capacity(vertex_count as usize);
        let normal_stream = &vertex_bytes[vertex_count as usize * 20..vertex_count as usize * 24];
        let tangent_stream = &vertex_bytes[vertex_count as usize * 24..vertex_count as usize * 28];
        let skin_stream = &vertex_bytes[vertex_count as usize * 28..];
        let skin_influences = if cpu_vertex_declaration == 9 { 4 } else { 8 };
        let mut geometry_valid = true;
        for (vertex_index, vertex) in primary.chunks_exact(primary_stride).enumerate() {
            let point = std::array::from_fn(|axis| {
                f32::from_le_bytes(vertex[axis * 4..axis * 4 + 4].try_into().unwrap())
            });
            if !point
                .iter()
                .all(|value: &f32| value.is_finite() && value.abs() < 100_000.0)
            {
                geometry_valid = false;
            }
            if geometry_valid {
                positions.push(point);
            }
            // Live declarations 9/10: streams 0=position,1=UV0/UV1,
            // 2=normal,3=tangent,4=weights/indices (4 or 8 influences).
            let uv = &texcoord_stream[vertex_index * 8..vertex_index * 8 + 8];
            let u = half::f16::from_bits(u16::from_le_bytes(uv[0..2].try_into().unwrap())).to_f32();
            let v = half::f16::from_bits(u16::from_le_bytes(uv[2..4].try_into().unwrap())).to_f32();
            if !u.is_finite() || !v.is_finite() {
                return Err(format!("surface {number} has non-finite UV0"));
            }
            if geometry_valid {
                uvs.push([u, v]);
                let mut packed = [0; 12];
                packed[..4].copy_from_slice(&uv[4..8]);
                packed[4..8]
                    .copy_from_slice(&normal_stream[vertex_index * 4..vertex_index * 4 + 4]);
                packed[8..12]
                    .copy_from_slice(&tangent_stream[vertex_index * 4..vertex_index * 4 + 4]);
                packed_channels.push(packed);
                let width = skin_influences as usize;
                let skin = &skin_stream[vertex_index * width * 2..(vertex_index + 1) * width * 2];
                let mut weights = [0; 8];
                let mut joints = [0; 8];
                weights[..width].copy_from_slice(&skin[..width]);
                joints[..width].copy_from_slice(&skin[width..]);
                skin_weights.push(weights);
                skin_indices.push(joints);
            }
        }
        if !geometry_valid {
            positions.clear();
            uvs.clear();
            packed_channels.clear();
            skin_weights.clear();
            skin_indices.clear();
        }
        let index_count = input.u32()?;
        if index_count > 3_000_000 {
            return Err(format!("surface {number} index count too large"));
        }
        let indices = input.take(
            (index_count as usize)
                .checked_mul(2)
                .ok_or("MD6 index size overflow")?,
        )?;
        for pair in indices.chunks_exact(2) {
            if u16::from_le_bytes(pair.try_into().unwrap()) as u32 >= vertex_count {
                return Err(format!("surface {number} index exceeds vertex count"));
            }
        }
        let indices = indices
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes(pair.try_into().unwrap()))
            .collect();
        input.take(28)?; // six bounds floats and one extent field
        let attachment_count = input.u32()?;
        if attachment_count > 100_000 {
            return Err(format!("surface {number} attachment count too large"));
        }
        input.take(
            (attachment_count as usize)
                .checked_mul(12)
                .ok_or("MD6 attachment size overflow")?,
        )?;
        input.take(48)?; // two fields and 40-byte per-surface state
        surfaces.push(SurfaceInfo {
            name,
            material_path,
            cpu_vertex_declaration,
            gpu_vertex_declaration,
            vertex_count,
            index_count,
            attachment_count,
            positions,
            uvs,
            packed_channels,
            skin_influences,
            skin_weights,
            skin_indices,
            indices,
        });
    }
    let margin = (0..3)
        .map(|axis| authored_bounds[1][axis] - authored_bounds[0][axis])
        .fold(0.0f32, f32::max)
        .max(1.0)
        * 0.02;
    let has_decoded_geometry = authored_bounds
        .iter()
        .flatten()
        .all(|value| value.is_finite())
        && surfaces.iter().all(|surface| {
            surface.vertex_count == 0
                || (!surface.positions.is_empty()
                    && surface.positions.iter().all(|point| {
                        (0..3).all(|axis| {
                            point[axis] >= authored_bounds[0][axis] - margin
                                && point[axis] <= authored_bounds[1][axis] + margin
                        })
                    }))
        });
    Ok(ModelInfo {
        surfaces,
        bytes_after_surfaces: bytes.len() - input.at,
        has_skeleton: !source_name.is_empty(),
        has_decoded_geometry,
        authored_bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_truncated_or_wrong_signature() {
        assert!(inspect(b"MM6B").is_err());
        assert!(inspect(b"BML1").is_err());
    }
    #[test]
    fn accepts_authored_empty_model_container() {
        let mut bytes = b"MM6B".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        let result = inspect(&bytes).unwrap();
        assert!(result.surfaces.is_empty());
        assert_eq!(result.bytes_after_surfaces, 0);
    }
    #[test]
    fn resolves_md6_declaration_mesh_name() {
        assert_eq!(
            compiled_mesh_path("lod 0 { mesh \"models/a/actual.md6mesh\" }"),
            Some("generated/basemodel/models/a/actual.bmd6model".into())
        );
        assert_eq!(
            declaration_path("generated/basemodel/models/a/alias.bmd6model"),
            Some("generated/decls/md6def/models/a/alias.md6.decl".into())
        );
    }
    #[test]
    fn resolves_inherited_lod_zero_mesh() {
        let path =
            resolve_compiled_mesh(
                "generated/basemodel/models/child.bmd6model",
                |decl| match decl {
                    "generated/decls/md6def/models/child.md6.decl" => {
                        Some("lod 0 { } inherit \"models/parent.md6\"".into())
                    }
                    "generated/decls/md6def/models/parent.md6.decl" => {
                        Some("lod 0 { mesh \"models/actual.md6mesh\" }".into())
                    }
                    _ => None,
                },
            )
            .unwrap();
        assert_eq!(path, "generated/basemodel/models/actual.bmd6model");
    }
    #[test]
    fn reads_planar_cpu_streams_for_both_vertex_declarations() {
        for (declaration, stride) in [(9u32, 36usize), (10, 44)] {
            let mut bytes = b"MM6B".to_vec();
            bytes.extend_from_slice(&0u32.to_le_bytes()); // no skeleton source
            for value in [0.0f32, 0.0, 0.0, 2.0, 1.0, 0.0] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            bytes.extend_from_slice(&1u32.to_le_bytes()); // one surface
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.push(b'x');
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.push(b'm');
            bytes.extend_from_slice(&[0; 9]); // surface fields
            bytes.extend_from_slice(b"BTRI");
            bytes.extend_from_slice(&0x0001_0002u32.to_le_bytes());
            bytes.extend_from_slice(&declaration.to_le_bytes());
            bytes.extend_from_slice(&declaration.to_le_bytes());
            bytes.extend_from_slice(&[0; 40]);
            bytes.extend_from_slice(&3u32.to_le_bytes());
            bytes.extend_from_slice(&0u64.to_le_bytes());
            for point in [[0.0f32, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
                for value in point {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
                for value in uv {
                    bytes.extend_from_slice(&half::f16::from_f32(value).to_bits().to_le_bytes());
                }
                bytes.extend_from_slice(&[0; 4]);
            }
            bytes.extend_from_slice(&vec![0; (stride - 20) * 3]);
            bytes.extend_from_slice(&3u32.to_le_bytes());
            for index in [0u16, 1, 2] {
                bytes.extend_from_slice(&index.to_le_bytes());
            }
            bytes.extend_from_slice(&[0; 28]);
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&[0; 48]);
            let model = inspect(&bytes).unwrap();
            assert!(model.has_decoded_geometry);
            assert_eq!(model.surfaces[0].positions[1], [2.0, 0.0, 0.0]);
            assert_eq!(model.surfaces[0].uvs[2], [0.0, 1.0]);
            assert_eq!(model.surfaces[0].indices, [0, 1, 2]);
        }
    }
}
