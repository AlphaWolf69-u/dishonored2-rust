//! Read the envelope and self-describing Havok tag sections in Void `.bphysworld`.
//! Format observations come from local game resources; tag section conventions
//! were cross-checked against MIT-licensed TagTools by Skyth.
use std::collections::BTreeMap;
pub mod mass;
pub mod physics_system;
pub mod compound;
pub mod body;
pub mod polytope;
pub mod shape;
pub mod query_geometry;
pub mod properties;
#[cfg(test)]
#[path = "physworld/shape_names_tests.rs"]
mod shape_names_tests;

#[derive(Debug)]
pub struct PhysWorld {
    pub materials: Vec<String>,
    pub material_map: Vec<u32>,
    pub sdk_version: String,
    pub types: Vec<TagType>,
    pub items: Vec<TagItem>,
    pub data_start: usize,
    pub data_size: usize,
}

#[derive(Debug)]
pub struct TagType {
    pub name: String,
    pub parent: usize,
    pub flags: u32,
    pub subtype: u32,
    pub pointer: usize,
    pub byte_size: usize,
    pub fields: Vec<TagField>,
}

#[derive(Debug)]
pub struct TagField {
    pub name: String,
    pub flags: u32,
    pub offset: usize,
    pub type_index: usize,
}

#[derive(Debug)]
pub struct TagItem {
    pub type_index: usize,
    pub offset: usize,
    pub count: usize,
    pub flags: u32,
}

/// A convex collider recovered from the authored Havok physics world.
/// Coordinates remain in the game's Z-up space.
#[derive(Debug)]
pub struct ConvexCollider {
    pub vertices: Vec<[f32; 3]>,
    pub source_shape_item: usize,
}

#[derive(Debug)]
pub struct CompressedCollider {
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    pub source_shape_item: usize,
}

#[derive(Debug)]
pub struct MeshTriangle {
    pub vertices: [[f32; 3]; 3],
    pub source_shape_item: usize,
    /// Shape tags along the triangle's shape-key path, as FUN_1400922d0
    /// hands them to the world's shape-tag codec: compound levels outermost
    /// first, then the leaf primitive's tag. Unused entries are NO_SHAPE_TAG,
    /// which the codec skips.
    pub shape_tags: ShapeTagPath,
    /// Index into `PhysWorld::materials` of the physics material the game
    /// resolves for this triangle's shape key (see `shape_tag_material`).
    /// `None` for standalone shapes and for codec paths whose result is a
    /// runtime material-library id rather than a world-table entry.
    pub material: Option<u32>,
}

/// The hknpMaterialId FUN_1400922d0 threads through the shape-tag codec:
/// it starts as the body's id and each decoded tag replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecMaterial {
    /// A raw id: the body's own, or `tag & 0x7fff` of an absolute tag. A
    /// following relative tag uses it as the base of its table index; as a
    /// final result it names a runtime hknpMaterialLibrary entry, which is
    /// not statically known.
    Raw(u16),
    /// An entry of the world table that FUN_140852100 fills with the
    /// library id of `materials[material_map[index]]`.
    Map(u32),
}

/// arkShapeTagCodecLocal::decode (FUN_140853b80, vtable 0x141c80060 slot 4)
/// for one tag: 0xffff leaves the id; bit 15 sets the id to `tag & 0x7fff`;
/// otherwise the id becomes the world table entry `(tag & 0x7fff) + id`.
/// A relative tag after a table hit would add a runtime library id, which is
/// not statically known. Level worlds use absolute compound-instance tags
/// as the base of their leaf tags.
pub fn decode_shape_tag(material: Option<CodecMaterial>, tag: u16) -> Option<CodecMaterial> {
    match (material?, tag) {
        (current, NO_SHAPE_TAG) => Some(current),
        (_, tag) if tag & 0x8000 != 0 => Some(CodecMaterial::Raw(tag & 0x7fff)),
        (CodecMaterial::Raw(id), tag) => {
            Some(CodecMaterial::Map((tag & 0x7fff) as u32 + id as u32))
        }
        (CodecMaterial::Map(_), _) => None,
    }
}

/// FUN_1400922d0: decode a body's hknpMaterialId through every tag on the
/// shape-key path, outermost first, and resolve a world-table result to its
/// index in `materials` (FUN_140852100 builds entry `i` from
/// `materials[material_map[i]]`).
pub fn shape_tag_material(
    body_material: u16,
    shape_tags: &ShapeTagPath,
    material_map: &[u32],
) -> Option<u32> {
    let decoded = shape_tags
        .iter()
        .fold(Some(CodecMaterial::Raw(body_material)), |material, &tag| {
            decode_shape_tag(material, tag)
        })?;
    match decoded {
        CodecMaterial::Map(index) => material_map.get(index as usize).copied(),
        CodecMaterial::Raw(_) => None,
    }
}

/// hknpShapeTag "invalid": FUN_140853b80 leaves the material unchanged.
pub const NO_SHAPE_TAG: u16 = 0xffff;
/// The shape-key collector in FUN_1400922d0 holds eight parent levels plus
/// the leaf tags (collector +0xc14).
pub const SHAPE_TAG_LEVELS: usize = 9;
pub type ShapeTagPath = [u16; SHAPE_TAG_LEVELS];
pub const NO_SHAPE_TAGS: ShapeTagPath = [NO_SHAPE_TAG; SHAPE_TAG_LEVELS];

#[derive(Default, Debug)]
pub struct CollisionExtraction {
    pub shape_bodies: BTreeMap<usize, Vec<usize>>,
    pub convex: Vec<ConvexCollider>,
    pub compressed: Vec<CompressedCollider>,
    pub triangles: Vec<MeshTriangle>,
    pub compressed_decode_failures: Vec<String>,
    pub unknown_shapes: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct BodyPose {
    pub position: [f32; 3],
    /// Havok quaternion in x, y, z, w order.
    pub orientation: [f32; 4],
}

#[derive(Clone, Copy)]
struct Section {
    tag: [u8; 4],
    content: usize,
    end: usize,
}

fn slice(bytes: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    bytes
        .get(at..at.checked_add(len).ok_or("offset overflow")?)
        .ok_or_else(|| format!("section at {at} of {len} bytes exceeds file"))
}
fn le32(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(slice(bytes, at, 4)?.try_into().unwrap()))
}
fn be32(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_be_bytes(slice(bytes, at, 4)?.try_into().unwrap()))
}
fn section(bytes: &[u8], at: usize) -> Result<Section, String> {
    let size = (be32(bytes, at)? & 0x3fff_ffff) as usize;
    if size < 8 {
        return Err(format!("short tag section at {at}"));
    }
    let end = at.checked_add(size).ok_or("tag section size overflow")?;
    slice(bytes, at, size)?;
    Ok(Section {
        tag: slice(bytes, at + 4, 4)?.try_into().unwrap(),
        content: at + 8,
        end,
    })
}
fn child(bytes: &[u8], parent: Section, wanted: &[u8; 4]) -> Result<Section, String> {
    let mut at = parent.content;
    while at < parent.end {
        let next = section(bytes, at)?;
        if next.end > parent.end {
            return Err("child extends beyond parent tag section".into());
        }
        if &next.tag == wanted {
            return Ok(next);
        }
        at = next.end;
    }
    Err(format!(
        "missing {} section",
        String::from_utf8_lossy(wanted)
    ))
}
fn packed(bytes: &[u8], at: &mut usize, end: usize) -> Result<usize, String> {
    let first = *slice(bytes, *at, 1)?.first().unwrap();
    *at += 1;
    let value = if first == 0xe8 {
        // Void's compendium uses an extended 32-bit form for hkHandle's
        // 0x7fffffff template value; the older TagTools reader lacks it.
        let value = be32(bytes, *at)? as usize;
        *at += 4;
        value
    } else if first & 0x80 == 0 {
        first as usize
    } else if first & 0x40 == 0 {
        let b = *slice(bytes, *at, 1)?.first().unwrap() as usize;
        *at += 1;
        ((first as usize) << 8 | b) & 0x3fff
    } else if first & 0x20 == 0 {
        let tail = u16::from_be_bytes(slice(bytes, *at, 2)?.try_into().unwrap()) as usize;
        *at += 2;
        ((first as usize) << 16 | tail) & 0x1f_ffff
    } else {
        let tail = *slice(bytes, *at, 1)?.first().unwrap() as usize;
        let final_word = u16::from_be_bytes(slice(bytes, *at + 1, 2)?.try_into().unwrap()) as usize;
        *at += 3;
        ((first as usize) << 24 | tail << 16 | final_word) & 0x1fff_ffff
    };
    if *at > end {
        return Err("packed integer exceeds tag section".into());
    }
    Ok(value)
}
fn strings(bytes: &[u8], section: Section) -> Result<Vec<String>, String> {
    slice(bytes, section.content, section.end - section.content)?
        .split(|&byte| byte == 0)
        .map(|value| String::from_utf8(value.to_vec()).map_err(|error| error.to_string()))
        .collect()
}
fn string_at(strings: &[String], index: usize) -> Result<String, String> {
    strings
        .get(index)
        .cloned()
        .ok_or_else(|| format!("string index {index} out of range"))
}

pub fn parse(bytes: &[u8]) -> Result<PhysWorld, String> {
    let material_count = le32(bytes, 0)? as usize;
    if material_count > 4096 {
        return Err("unreasonable material count".into());
    }
    let mut at = 4usize;
    let mut materials = Vec::with_capacity(material_count);
    for _ in 0..material_count {
        let length = le32(bytes, at)? as usize;
        at += 4;
        if length > 4096 {
            return Err("unreasonable material path length".into());
        }
        materials.push(
            std::str::from_utf8(slice(bytes, at, length)?)
                .map_err(|error| error.to_string())?
                .to_string(),
        );
        at += length;
    }
    let map_count = le32(bytes, at)? as usize;
    at += 4;
    if map_count > 1_000_000 {
        return Err("unreasonable material map count".into());
    }
    let mut material_map = Vec::with_capacity(map_count);
    for _ in 0..map_count {
        let material = le32(bytes, at)?;
        if material as usize >= material_count {
            return Err("material map index out of range".into());
        }
        material_map.push(material);
        at += 4;
    }
    let compendium_size = le32(bytes, at)? as usize;
    at += 4;
    let compendium = section(bytes, at)?;
    if compendium.tag != *b"TCM0" || compendium.end - at != compendium_size {
        return Err("invalid Havok type compendium".into());
    }
    at = compendium.end;
    let tag_size = le32(bytes, at)? as usize;
    at += 4;
    let tag = section(bytes, at)?;
    if tag.tag != *b"TAG0" || tag.end - at != tag_size || tag.end != bytes.len() {
        return Err("invalid Havok object tag file".into());
    }
    let type_section = child(bytes, compendium, b"TYPE")?;
    parse_tagged(
        bytes,
        materials,
        material_map,
        tag,
        type_section,
        Some(compendium),
    )
}

/// Standalone model/blueprint `.hkx` resources carry their TYPE section inside
/// TAG0 instead of referencing a separate TCM0 compendium.
pub fn parse_hkx(bytes: &[u8]) -> Result<PhysWorld, String> {
    let tag_offset = (4..=16)
        .find(|&at| bytes.get(at + 4..at + 8) == Some(b"TAG0"))
        .ok_or("standalone HKX has no TAG0 section")?;
    let prefix_kind = bytes
        .get(..4)
        .and_then(|head| head.try_into().ok())
        .map(u32::from_le_bytes);
    if !matches!(prefix_kind, Some(0 | 2)) || bytes[4..tag_offset].iter().any(|&byte| byte != 0) {
        return Err("unexpected HKX prefix".into());
    }
    let tag = section(bytes, tag_offset)?;
    if tag.tag != *b"TAG0" || tag.end != bytes.len() {
        return Err("invalid standalone Havok tag file".into());
    }
    let type_section = child(bytes, tag, b"TYPE")?;
    parse_tagged(bytes, Vec::new(), Vec::new(), tag, type_section, None)
}

/// Navmesh containers embed the same Havok TAG0 stream after a layer header.
/// Parsing it provides type/geometry evidence; it is not player collision.
pub fn parse_embedded_havok(bytes: &[u8]) -> Result<PhysWorld, String> {
    let tag_offset = bytes
        .windows(4)
        .take(1024)
        .position(|window| window == b"TAG0")
        .and_then(|position| position.checked_sub(4))
        .ok_or("no embedded Havok TAG0 stream")?;
    let tag = section(bytes, tag_offset)?;
    let type_section = child(bytes, tag, b"TYPE")?;
    parse_tagged(bytes, Vec::new(), Vec::new(), tag, type_section, None)
}

/// Some scaled HKX archive entries are small redirects to a base collision
/// resource. The uniform factor is applied to the base resource's vertices.
pub fn standalone_hkx_alias(bytes: &[u8]) -> Option<(String, f32)> {
    if bytes.len() < 13 || bytes[..5] != [0, 0, 0, 0, 1] {
        return None;
    }
    let factor = f32::from_le_bytes(bytes[5..9].try_into().ok()?);
    let length = u32::from_le_bytes(bytes[9..13].try_into().ok()?) as usize;
    if !factor.is_finite() || factor <= 0.0 || 13 + length != bytes.len() {
        return None;
    }
    let target = std::str::from_utf8(&bytes[13..]).ok()?;
    if !target.starts_with("models/")
        || ![".hkt._", ".cvx._", ".box._"]
            .iter()
            .any(|kind| target.contains(kind))
    {
        return None;
    }
    Some((physics_shape_asset_path(target), factor))
}

/// FUN_140143650: signed-byte C-string ordering with ASCII-only case folding.
fn ascii_cstr_compare(left: &[u8], right: &[u8]) -> i32 {
    for index in 0.. {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        let fold = |byte: u8| {
            if byte.is_ascii_uppercase() {
                byte + 32
            } else {
                byte
            }
        };
        let difference = (fold(a) as i8 as i32) - (fold(b) as i8 as i32);
        if difference != 0 {
            return difference.signum();
        }
        if a == 0 {
            return 0;
        }
    }
    unreachable!()
}

/// FUN_14008c680: suffix kinds 0=hkt, 1=cvx, 2=msh, 3=box, 4=other.
pub fn physics_shape_kind(suffix: &str) -> u8 {
    ["hkt", "cvx", "msh", "box"]
        .iter()
        .position(|kind| ascii_cstr_compare(suffix.as_bytes(), kind.as_bytes()) == 0)
        .map_or(4, |index| index as u8)
}

/// FUN_140145000 stops at either path separator when removing an extension.
pub(crate) fn strip_shape_extension(path: &str) -> &str {
    match path.rfind(['.', '/', '\\']) {
        Some(index) if path.as_bytes()[index] == b'.' => &path[..index],
        _ => path,
    }
}

/// 00ee40/142150 scans bytes before the final character for the last dot,
/// including dots before path separators. The search skips a final dot.
pub fn shape_extension(path: &str) -> &str {
    let path = path.split('\0').next().unwrap();
    path.as_bytes()[..path.len().saturating_sub(1)]
        .iter()
        .rposition(|&byte| byte == b'.')
        .map_or("", |dot| &path[dot + 1..])
}

/// FUN_140144e50 replaces an extension and accepts a leading dot in the new one.
pub fn replace_shape_extension(path: &str, extension: &str) -> String {
    let extension = extension.split('\0').next().unwrap();
    let dot = if extension.starts_with('.') { "" } else { "." };
    format!("{}{dot}{extension}", strip_shape_extension(path))
}

/// FUN_14008c010 restores the encoded _hd marker before replacing the extension.
pub fn hkt_shape_name(name: &str, hd: bool) -> String {
    let name = if hd {
        format!("{}_hd", strip_shape_extension(name))
    } else {
        name.to_owned()
    };
    replace_shape_extension(&name, "hkt")
}

/// FUN_14008c330 recipe value. Unsupported enum values and floorf's unported
/// NaN dispatcher are explicit failures; HD preserves the original extension.
pub fn physics_shape_key(
    name: &str,
    kind: u8,
    scale: [f32; 3],
    radius: f32,
    hd: bool,
) -> Option<String> {
    let suffix = ["hkt", "cvx", "msh", "box", "cvd"].get(kind as usize)?;
    if scale.into_iter().chain([radius]).any(f32::is_nan) {
        return None;
    }
    // Original MULSS, ADDSS, floorf, CVTTSS2SI. Rust's saturating cast alone
    // is not equivalent: out-of-range values convert to INT_MIN.
    let quantize = |value: f32, factor: f32| {
        let rounded = (value * factor + 0.5).floor();
        if rounded < -2147483648.0 || rounded >= 2147483648.0 {
            i32::MIN
        } else {
            rounded as i32
        }
    };
    let [x, y, z] = scale.map(|value| {
        let integer = quantize(value, 100.0);
        if integer == 0 {
            // FUN_140166830 only homes its variadic arguments and returns.
            if value < 0.0 {
                -1
            } else {
                1
            }
        } else {
            integer
        }
    });
    let radius = quantize(radius, 1000.0);
    let name = name.split('\0').next().unwrap();
    let hd_name;
    let name = if hd {
        // Extraction scans all dots; stripping stops at a path separator.
        let extension = shape_extension(name);
        hd_name = format!("{}_hd.{extension}", strip_shape_extension(name));
        &hd_name
    } else {
        name
    };
    Some(format!("{name}.{suffix}._{x}_{y}_{z}.{radius}"))
}

/// FUN_14008bef0 formats the stored shape key, without parsing or resolving it.
pub fn physics_shape_asset_path(key: &str) -> String {
    format!("generated/{}.hkx", key.split('\0').next().unwrap())
}

/// FUN_140aeb340: matrix columns to quaternion, including original branch ties.
pub fn basis_quaternion(basis: [[f32; 4]; 3]) -> [f32; 4] {
    normalize_body_quaternion(rotation_quaternion(basis))
}

/// FUN_140aea690 converts a rotation basis without the normalization performed
/// by 140aeb340. Constraint motion history uses this distinct entry point.
pub fn rotation_quaternion(basis: [[f32; 4]; 3]) -> [f32; 4] {
    let trace = (basis[0][0] + basis[1][1]) + basis[2][2];
    let mut quaternion = [0.0; 4];
    if trace > 0.0 {
        let root = (trace + 1.0).sqrt();
        let factor = 0.5 / root;
        quaternion[3] = root * 0.5;
        quaternion[0] = (basis[1][2] - basis[2][1]) * factor;
        quaternion[1] = (basis[2][0] - basis[0][2]) * factor;
        quaternion[2] = (basis[0][1] - basis[1][0]) * factor;
    } else {
        let mut largest = usize::from(basis[1][1] > basis[0][0]);
        if basis[2][2] > basis[largest][largest] {
            largest = 2;
        }
        let next = [1, 2, 0];
        let second = next[largest];
        let third = next[second];
        let root = ((basis[largest][largest] - (basis[third][third] + basis[second][second]))
            + 1.0)
            .sqrt();
        let factor = 0.5 / root;
        quaternion[largest] = root * 0.5;
        quaternion[3] = (basis[second][third] - basis[third][second]) * factor;
        quaternion[second] = (basis[largest][second] + basis[second][largest]) * factor;
        quaternion[third] = (basis[largest][third] + basis[third][largest]) * factor;
    }
    quaternion
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn normalize_body_quaternion(quaternion: [f32; 4]) -> [f32; 4] {
    use std::arch::x86_64::*;
    // Exact RSQRTPS/Newton sequence; generic quaternion normalization differs.
    unsafe {
        let q = _mm_loadu_ps(quaternion.as_ptr());
        let squares = _mm_mul_ps(q, q);
        let pairs = _mm_add_ps(_mm_shuffle_ps::<0x4e>(squares, squares), squares);
        let length = _mm_add_ps(_mm_shuffle_ps::<0xb1>(pairs, pairs), pairs);
        let estimate = _mm_rsqrt_ps(length);
        let error = _mm_mul_ps(_mm_mul_ps(estimate, length), estimate);
        let correction = _mm_mul_ps(
            _mm_sub_ps(_mm_set1_ps(3.0), error),
            _mm_mul_ps(estimate, _mm_set1_ps(0.5)),
        );
        let mut result = [0.0; 4];
        _mm_storeu_ps(result.as_mut_ptr(), _mm_mul_ps(q, correction));
        result
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub(crate) fn normalize_body_quaternion(_quaternion: [f32; 4]) -> [f32; 4] {
    unimplemented!("DH2 body normalization requires x86-64 RSQRTPS")
}

/// FUN_140aa0390: quaternion to all twelve matrix lanes, including padding.
#[cfg(target_arch = "x86_64")]
pub fn quaternion_basis(quaternion: [f32; 4]) -> [[f32; 4]; 3] {
    use std::arch::x86_64::*;
    // Packed operation order and sign-mask XORs preserve native NaN bits too.
    unsafe {
        let q = _mm_loadu_ps(quaternion.as_ptr());
        let sign = _mm_castsi128_ps(_mm_set_epi32(i32::MIN, i32::MIN, i32::MIN, 0));
        let cross_sign = _mm_castsi128_ps(_mm_set_epi32(i32::MIN, i32::MIN, 0, i32::MIN));
        let one = _mm_set_ss(1.0);
        let doubled = _mm_add_ps(q, q);
        let xx = _mm_mul_ss(q, doubled);
        let products = _mm_mul_ps(
            _mm_shuffle_ps::<0xa5>(doubled, doubled),
            _mm_shuffle_ps::<0x41>(q, q),
        );
        let cross = _mm_mul_ps(
            _mm_shuffle_ps::<0xfe>(q, q),
            _mm_shuffle_ps::<0x1a>(doubled, doubled),
        );
        let mut signed_products = _mm_xor_ps(sign, products);
        let diagonal = _mm_sub_ss(_mm_sub_ss(one, xx), products);
        let cross = _mm_add_ss(_mm_xor_ps(cross, cross_sign), one);
        let first = _mm_sub_ps(cross, signed_products);
        signed_products = _mm_move_ss(signed_products, xx);
        let second = _mm_sub_ps(_mm_xor_ps(sign, cross), signed_products);
        let second = _mm_shuffle_ps::<0xb1>(second, second);
        let third = _mm_shuffle_ps::<0x8d>(_mm_movehl_ps(first, second), diagonal);
        let mut result = [[0.0; 4]; 3];
        for (column, value) in result.iter_mut().zip([first, second, third]) {
            _mm_storeu_ps(column.as_mut_ptr(), value);
        }
        result
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn quaternion_basis(_quaternion: [f32; 4]) -> [[f32; 4]; 3] {
    unimplemented!("DH2 basis conversion requires x86-64 SSE")
}

/// FUN_140aa0360: the original basis/quaternion/basis round trip before body creation.
pub fn rigid_body_basis(basis: [[f32; 4]; 3]) -> [[f32; 4]; 3] {
    quaternion_basis(basis_quaternion(basis))
}

fn parse_tagged(
    bytes: &[u8],
    materials: Vec<String>,
    material_map: Vec<u32>,
    tag: Section,
    type_section: Section,
    compendium: Option<Section>,
) -> Result<PhysWorld, String> {
    let sdk = child(bytes, tag, b"SDKV")?;
    let sdk_version = std::str::from_utf8(slice(bytes, sdk.content, sdk.end - sdk.content)?)
        .map_err(|error| error.to_string())?
        .to_string();
    let data = child(bytes, tag, b"DATA")?;
    let type_strings = strings(bytes, child(bytes, type_section, b"TSTR")?)?;
    let field_strings = strings(bytes, child(bytes, type_section, b"FSTR")?)?;
    let names =
        child(bytes, type_section, b"TNAM").or_else(|_| child(bytes, type_section, b"TNA1"))?;
    let mut position = names.content;
    let type_count = packed(bytes, &mut position, names.end)?;
    if type_count == 0 || type_count > 100_000 {
        return Err("unreasonable Havok type count".into());
    }
    let mut types = Vec::with_capacity(type_count);
    types.push(TagType {
        name: String::new(),
        parent: 0,
        flags: 0,
        subtype: 0,
        pointer: 0,
        byte_size: 0,
        fields: Vec::new(),
    });
    for type_number in 1..type_count {
        let name_index = packed(bytes, &mut position, names.end)?;
        let name = string_at(&type_strings, name_index)
            .map_err(|error| format!("TNAM type {type_number} at {position}: {error}"))?;
        let template_count = packed(bytes, &mut position, names.end)?;
        if template_count > 100 {
            return Err("unreasonable Havok template count".into());
        }
        for _ in 0..template_count {
            packed(bytes, &mut position, names.end)?;
            packed(bytes, &mut position, names.end)?;
        }
        types.push(TagType {
            name,
            parent: 0,
            flags: 0,
            subtype: 0,
            pointer: 0,
            byte_size: 0,
            fields: Vec::new(),
        });
    }
    let bodies =
        child(bytes, type_section, b"TBOD").or_else(|_| child(bytes, type_section, b"TBDY"))?;
    position = bodies.content;
    while position < bodies.end {
        let index = packed(bytes, &mut position, bodies.end)?;
        if index == 0 {
            continue;
        }
        let parent = packed(bytes, &mut position, bodies.end)?;
        let flags = packed(bytes, &mut position, bodies.end)? as u32;
        if index >= types.len() || parent >= types.len() {
            return Err("Havok type index out of range".into());
        }
        let mut subtype = 0;
        let mut pointer = 0;
        if flags & 1 != 0 {
            subtype = packed(bytes, &mut position, bodies.end)? as u32;
        }
        if flags & 2 != 0 && subtype & 0xf >= 6 {
            pointer = packed(bytes, &mut position, bodies.end)?;
        }
        if flags & 4 != 0 {
            packed(bytes, &mut position, bodies.end)?;
        }
        let mut byte_size = 0;
        if flags & 8 != 0 {
            byte_size = packed(bytes, &mut position, bodies.end)?;
            packed(bytes, &mut position, bodies.end)?;
        }
        if flags & 16 != 0 {
            packed(bytes, &mut position, bodies.end)?;
        }
        let mut fields = Vec::new();
        if flags & 32 != 0 {
            let count = packed(bytes, &mut position, bodies.end)?;
            if count > 1000 {
                return Err("unreasonable Havok field count".into());
            }
            for _ in 0..count {
                let name_index = packed(bytes, &mut position, bodies.end)?;
                let name = string_at(&field_strings, name_index)
                    .map_err(|error| format!("TBOD type {index} at {position}: {error}"))?;
                let field_flags = packed(bytes, &mut position, bodies.end)? as u32;
                let offset = packed(bytes, &mut position, bodies.end)?;
                let type_index = packed(bytes, &mut position, bodies.end)?;
                if type_index >= types.len() {
                    return Err("Havok field type out of range".into());
                }
                fields.push(TagField {
                    name,
                    flags: field_flags,
                    offset,
                    type_index,
                });
            }
        }
        if flags & 64 != 0 {
            let count = packed(bytes, &mut position, bodies.end)?;
            if count > 1000 {
                return Err("unreasonable Havok interface count".into());
            }
            for _ in 0..count {
                packed(bytes, &mut position, bodies.end)?;
                packed(bytes, &mut position, bodies.end)?;
            }
        }
        if flags & 128 != 0 {
            return Err("unsupported Havok type flag 0x80".into());
        }
        types[index].parent = parent;
        types[index].flags = flags;
        types[index].subtype = subtype;
        types[index].pointer = pointer;
        types[index].byte_size = byte_size;
        types[index].fields = fields;
    }
    if let Some(compendium) = compendium {
        let type_ref = match child(bytes, tag, b"TCRF") {
            Ok(reference) => reference,
            Err(_) => child(bytes, child(bytes, tag, b"TYPE")?, b"TCRF")?,
        };
        let compendium_id = slice(bytes, child(bytes, compendium, b"TCID")?.content, 8)?;
        if slice(bytes, type_ref.content, 8)? != compendium_id {
            return Err("tag file references a different Havok compendium".into());
        }
    }
    let item_section = child(bytes, child(bytes, tag, b"INDX")?, b"ITEM")?;
    if (item_section.end - item_section.content) % 12 != 0 {
        return Err("Havok item table is not 12-byte aligned".into());
    }
    let mut items = Vec::new();
    for item_at in (item_section.content..item_section.end).step_by(12) {
        let flags = le32(bytes, item_at)?;
        let type_index = (flags & 0x00ff_ffff) as usize;
        let offset = le32(bytes, item_at + 4)? as usize;
        let count = le32(bytes, item_at + 8)? as usize;
        if type_index >= types.len() || offset > data.end - data.content {
            return Err("Havok item references invalid type or data offset".into());
        }
        items.push(TagItem {
            type_index,
            offset,
            count,
            flags,
        });
    }
    Ok(PhysWorld {
        materials,
        material_map,
        sdk_version,
        types,
        items,
        data_start: data.content,
        data_size: data.end - data.content,
    })
}

impl PhysWorld {
    fn field(&self, mut type_index: usize, wanted: &str) -> Result<&TagField, String> {
        for _ in 0..self.types.len() {
            let ty = self
                .types
                .get(type_index)
                .ok_or("invalid Havok type index")?;
            if let Some(field) = ty.fields.iter().find(|field| field.name == wanted) {
                return Ok(field);
            }
            if ty.parent == 0 {
                break;
            }
            type_index = ty.parent;
        }
        Err(format!("Havok type has no {wanted} field"))
    }

    fn field_offset(&self, type_index: usize, wanted: &str) -> Result<usize, String> {
        Ok(self.field(type_index, wanted)?.offset)
    }

    fn compressed_tree<'a>(
        &self,
        bytes: &'a [u8],
        shape_index: usize,
    ) -> Result<(&'a [u8], usize), String> {
        let shape_type = self
            .items
            .get(shape_index)
            .ok_or("missing compressed shape")?
            .type_index;
        let shape = self.item_at(bytes, shape_index, 0)?;
        let data_ref = le32(shape, self.field_offset(shape_type, "data")?)? as usize;
        let data_type = self
            .items
            .get(data_ref)
            .ok_or("missing compressed mesh data")?
            .type_index;
        let data = self.item_at(bytes, data_ref, 0)?;
        let tree = self.field(data_type, "meshTree")?;
        Ok((
            slice(data, tree.offset, self.type_size(tree.type_index)?)?,
            tree.type_index,
        ))
    }

    fn compressed_domain(
        &self,
        tree: &[u8],
        tree_type: usize,
    ) -> Result<([f32; 3], [f32; 3]), String> {
        let domain = self.field(tree_type, "domain")?;
        let low = domain.offset + self.field_offset(domain.type_index, "min")?;
        let high = domain.offset + self.field_offset(domain.type_index, "max")?;
        Ok((
            [
                float(tree, low)?,
                float(tree, low + 4)?,
                float(tree, low + 8)?,
            ],
            [
                float(tree, high)?,
                float(tree, high + 4)?,
                float(tree, high + 8)?,
            ],
        ))
    }
    /// Decode a standalone model/blueprint HKX whose item 1 is its root shape.
    pub fn standalone_shapes(&self, bytes: &[u8]) -> Result<CollisionExtraction, String> {
        let root = self.items.get(1).ok_or("standalone HKX has no root item")?;
        let name = &self.types[root.type_index].name;
        if !name.ends_with("Shape") {
            return Err(format!("standalone HKX root is {name}, not a shape"));
        }
        let mut output = CollisionExtraction::default();
        self.visit_shape(
            bytes,
            1,
            Affine::translated([0.0; 3]),
            0,
            NO_SHAPE_TAGS,
            &mut output,
        )?;
        Ok(output)
    }
    /// Count primitive records that can form a nondegenerate triangle before
    /// decoding vertices. Useful for distinguishing line/point proxies from
    /// missing surface geometry in failed compressed meshes.
    pub fn compressed_surface_primitives(
        &self,
        bytes: &[u8],
        shape_index: usize,
    ) -> Result<(usize, usize), String> {
        let (tree, tree_type) = self.compressed_tree(bytes, shape_index)?;
        let sections_ref = le32(tree, self.field_offset(tree_type, "sections")?)? as usize;
        let primitives_ref = le32(tree, self.field_offset(tree_type, "primitives")?)? as usize;
        let section_count = self
            .items
            .get(sections_ref)
            .ok_or("missing section item")?
            .count;
        let mut all = 0usize;
        let mut surfaces = 0usize;
        for section_number in 0..section_count {
            let section = self.item_at(bytes, sections_ref, section_number)?;
            let header = le32(section, 0x50)? as usize;
            let first = header >> 8;
            let count = header & 0xff;
            for primitive_number in first..first + count {
                let primitive = self.item_at(bytes, primitives_ref, primitive_number)?;
                all += 1;
                if primitive_has_surface(primitive) {
                    surfaces += 1;
                }
            }
        }
        Ok((all, surfaces))
    }

    pub fn item_inventory(&self) -> BTreeMap<&str, usize> {
        let mut counts = BTreeMap::new();
        for item in &self.items {
            if item.type_index != 0 {
                *counts
                    .entry(self.types[item.type_index].name.as_str())
                    .or_insert(0) += 1;
            }
        }
        counts
    }

    fn type_size(&self, mut index: usize) -> Result<usize, String> {
        for _ in 0..self.types.len() {
            let typ = self.types.get(index).ok_or("invalid type index")?;
            if typ.byte_size != 0 {
                return Ok(typ.byte_size);
            }
            if typ.parent == 0 {
                break;
            }
            index = typ.parent;
        }
        Err("Havok type has no concrete byte size".into())
    }

    fn item_at<'a>(
        &self,
        bytes: &'a [u8],
        index: usize,
        element: usize,
    ) -> Result<&'a [u8], String> {
        let item = self
            .items
            .get(index)
            .ok_or_else(|| format!("invalid item reference {index}"))?;
        if element >= item.count {
            return Err(format!("item {index} element {element} out of range"));
        }
        let size = self.type_size(item.type_index)?;
        let start = self.data_start + item.offset + element * size;
        if start + size > self.data_start + self.data_size {
            return Err("item exceeds DATA".into());
        }
        slice(bytes, start, size)
    }

    pub fn body_poses(&self, bytes: &[u8]) -> Result<Vec<BodyPose>, String> {
        let root = self.item_at(bytes, 1, 0)?;
        let bodies = le32(root, 0x38)? as usize;
        let body_item = self.items.get(bodies).ok_or("missing body item")?;
        let mut poses = Vec::with_capacity(body_item.count);
        for body in 0..body_item.count {
            let info = self.item_at(bytes, bodies, body)?;
            let position = [float(info, 0x30)?, float(info, 0x34)?, float(info, 0x38)?];
            let orientation = [
                float(info, 0x40)?,
                float(info, 0x44)?,
                float(info, 0x48)?,
                float(info, 0x4c)?,
            ];
            poses.push(BodyPose {
                position,
                orientation,
            });
        }
        Ok(poses)
    }

    pub fn collision_shapes(&self, bytes: &[u8]) -> Result<CollisionExtraction, String> {
        let root = self.item_at(bytes, 1, 0)?;
        let bodies = le32(root, 0x38)? as usize;
        let body_item = self.items.get(bodies).ok_or("missing body item")?;
        // hknpPhysicsSystemData::materials (+0x18). Level worlds carry none,
        // so a body's serialized materialId is the id the codec starts from;
        // with system materials the id would be remapped at insertion.
        let system_materials = le32(root, 0x18)? as usize;
        let body_ids_are_raw = system_materials == 0
            || self
                .items
                .get(system_materials)
                .is_none_or(|item| item.count == 0);
        let mut output = CollisionExtraction::default();
        for body in 0..body_item.count {
            let info = self.item_at(bytes, bodies, body)?;
            let shape = le32(info, 0)? as usize;
            if shape == 0 {
                continue;
            }
            // All tutorial physics-system body origins are zero; preserve the
            // per-body translation for other worlds. Quaternion support is
            // deferred until a rotated body appears in a tested resource.
            let position = [float(info, 0x30)?, float(info, 0x34)?, float(info, 0x38)?];
            let body_material = u16::from_le_bytes(
                slice(
                    info,
                    self.field_offset(body_item.type_index, "materialId")?,
                    2,
                )?
                .try_into()
                .unwrap(),
            );
            let first = output.triangles.len();
            self.visit_shape(
                bytes,
                shape,
                Affine::translated(position),
                0,
                NO_SHAPE_TAGS,
                &mut output,
            )?;
            if body_ids_are_raw {
                for triangle in &mut output.triangles[first..] {
                    triangle.material =
                        shape_tag_material(body_material, &triangle.shape_tags, &self.material_map);
                }
            }
            for triangle in &output.triangles[first..] {
                let bodies = output
                    .shape_bodies
                    .entry(triangle.source_shape_item)
                    .or_default();
                if bodies.last() != Some(&body) {
                    bodies.push(body);
                }
            }
        }
        Ok(output)
    }

    /// `depth` counts the compound levels above `index`, whose instance tags
    /// fill `shape_tags[..depth]`; a leaf writes its tag at `shape_tags[depth]`.
    #[allow(clippy::too_many_arguments)]
    fn visit_shape(
        &self,
        bytes: &[u8],
        index: usize,
        transform: Affine,
        depth: usize,
        shape_tags: ShapeTagPath,
        output: &mut CollisionExtraction,
    ) -> Result<(), String> {
        if depth >= SHAPE_TAG_LEVELS {
            return Err(format!(
                "nested collision shape depth exceeds {}",
                SHAPE_TAG_LEVELS - 1
            ));
        }
        let item = self
            .items
            .get(index)
            .ok_or_else(|| format!("invalid shape item {index}"))?;
        let name = self.types[item.type_index].name.as_str();
        let shape = self.item_at(bytes, index, 0)?;
        match name {
            "hknpConvexShape" => {
                let vertices_ref =
                    le32(shape, self.field_offset(item.type_index, "vertices")?)? as usize;
                let vertex_item = self
                    .items
                    .get(vertices_ref)
                    .ok_or("missing generic convex vertices")?;
                if self.types[vertex_item.type_index].name != "hkVector4f" {
                    return Err(format!(
                        "generic convex item {index} has unexpected vertex type"
                    ));
                }
                let mut vertices = Vec::with_capacity(vertex_item.count);
                for element in 0..vertex_item.count {
                    let source = self.item_at(bytes, vertices_ref, element)?;
                    vertices.push(transform.apply([
                        float(source, 0)?,
                        float(source, 4)?,
                        float(source, 8)?,
                    ]));
                }
                if vertices.len() == 4 {
                    let order = convex_quad_order(&vertices)?;
                    for corners in [
                        [order[0], order[1], order[2]],
                        [order[0], order[2], order[3]],
                    ] {
                        output.triangles.push(MeshTriangle {
                            vertices: corners.map(|corner| vertices[corner]),
                            source_shape_item: index,
                            shape_tags,
                            material: None,
                        });
                    }
                    output.convex.push(ConvexCollider {
                        vertices,
                        source_shape_item: index,
                    });
                } else {
                    *output
                        .unknown_shapes
                        .entry(format!("{name}({} vertices)", vertices.len()))
                        .or_insert(0) += 1;
                }
            }
            "hknpCompoundShape" => {
                let instances =
                    le32(shape, self.field_offset(item.type_index, "instances")?)? as usize;
                if instances == 0 {
                    return Ok(());
                }
                let list = self
                    .items
                    .get(instances)
                    .ok_or("missing shape instance array")?;
                if self.types[list.type_index].name != "hkFreeListArrayElement" {
                    return Err(format!(
                        "compound item {index} has unexpected instance type"
                    ));
                }
                for element in 0..list.count {
                    let instance = self.item_at(bytes, instances, element)?;
                    let child = le32(instance, 0x50)? as usize;
                    if child == 0 {
                        continue;
                    }
                    let local = Affine::from_havok_instance(instance)?;
                    // hknpShapeInstance::shapeTag (+0x58) is this level's tag.
                    let mut child_tags = shape_tags;
                    child_tags[depth] =
                        u16::from_le_bytes(slice(instance, 0x58, 2)?.try_into().unwrap());
                    self.visit_shape(
                        bytes,
                        child,
                        transform.compose(local),
                        depth + 1,
                        child_tags,
                        output,
                    )?;
                }
            }
            "hknpConvexPolytopeShape" | "hknpBoxShape" => {
                let vertices_ref =
                    le32(shape, self.field_offset(item.type_index, "vertices")?)? as usize;
                let vertex_item = self
                    .items
                    .get(vertices_ref)
                    .ok_or("missing convex vertex item")?;
                if self.types[vertex_item.type_index].name != "hkVector4f" {
                    return Err(format!("convex item {index} has unexpected vertex type"));
                }
                let mut vertices = Vec::with_capacity(vertex_item.count);
                for element in 0..vertex_item.count {
                    let source = self.item_at(bytes, vertices_ref, element)?;
                    vertices.push(transform.apply([
                        float(source, 0)?,
                        float(source, 4)?,
                        float(source, 8)?,
                    ]));
                }
                let faces_ref = le32(shape, self.field_offset(item.type_index, "faces")?)? as usize;
                let indices_ref =
                    le32(shape, self.field_offset(item.type_index, "indices")?)? as usize;
                let face_count = self
                    .items
                    .get(faces_ref)
                    .ok_or("missing convex face item")?
                    .count;
                let index_count = self
                    .items
                    .get(indices_ref)
                    .ok_or("missing convex index item")?
                    .count;
                for face_number in 0..face_count {
                    let face = self.item_at(bytes, faces_ref, face_number)?;
                    let first = u16::from_le_bytes(face[..2].try_into().unwrap()) as usize;
                    let count = face[2] as usize;
                    if count < 3 || first + count > index_count {
                        return Err(format!(
                            "convex item {index} face {face_number} has invalid index range"
                        ));
                    }
                    let mut corners = Vec::with_capacity(count);
                    for offset in first..first + count {
                        let vertex_index = self.item_at(bytes, indices_ref, offset)?[0] as usize;
                        corners.push(*vertices.get(vertex_index).ok_or_else(|| {
                            format!("convex item {index} has invalid vertex index")
                        })?);
                    }
                    for triangle in 1..count - 1 {
                        output.triangles.push(MeshTriangle {
                            vertices: [corners[0], corners[triangle], corners[triangle + 1]],
                            source_shape_item: index,
                            shape_tags,
                            material: None,
                        });
                    }
                }
                output.convex.push(ConvexCollider {
                    vertices,
                    source_shape_item: index,
                });
            }
            "hknpCompressedMeshShape" => {
                // Standalone HKX and level BPHYSWORLD use different serialized
                // pointer/array layouts; the authored type metadata owns offsets.
                let (tree, tree_type) = self.compressed_tree(bytes, index)?;
                let (min, max) = self.compressed_domain(tree, tree_type)?;
                let mut bounds_min = [f32::INFINITY; 3];
                let mut bounds_max = [f32::NEG_INFINITY; 3];
                for corner in 0..8 {
                    let local = std::array::from_fn(|axis| {
                        if corner & (1 << axis) == 0 {
                            min[axis]
                        } else {
                            max[axis]
                        }
                    });
                    let world = transform.apply(local);
                    for axis in 0..3 {
                        bounds_min[axis] = bounds_min[axis].min(world[axis]);
                        bounds_max[axis] = bounds_max[axis].max(world[axis]);
                    }
                }
                output.compressed.push(CompressedCollider {
                    bounds_min,
                    bounds_max,
                    source_shape_item: index,
                });
                let previous_triangles = output.triangles.len();
                let previous_convex = output.convex.len();
                if let Err(error) = self.decode_compressed_mesh(
                    bytes, tree, tree_type, index, transform, depth, shape_tags, output,
                ) {
                    output.triangles.truncate(previous_triangles);
                    output.convex.truncate(previous_convex);
                    output.compressed_decode_failures.push(error);
                }
            }
            _ => *output.unknown_shapes.entry(name.to_owned()).or_insert(0) += 1,
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_compressed_mesh(
        &self,
        bytes: &[u8],
        tree: &[u8],
        tree_type: usize,
        shape_index: usize,
        transform: Affine,
        depth: usize,
        shape_tags: ShapeTagPath,
        output: &mut CollisionExtraction,
    ) -> Result<(), String> {
        let references: Vec<usize> = [
            "sections",
            "primitives",
            "sharedVerticesIndex",
            "packedVertices",
            "sharedVertices",
        ]
        .into_iter()
        .map(|name| le32(tree, self.field_offset(tree_type, name)?).map(|value| value as usize))
        .collect::<Result<_, _>>()?;
        let [sections_ref, primitives_ref, shared_index_ref, packed_ref, shared_ref] =
            references.try_into().unwrap();
        let data_runs_ref =
            le32(tree, self.field_offset(tree_type, "primitiveDataRuns")?)? as usize;
        let data_runs: Vec<[u8; 4]> =
            (0..self.items.get(data_runs_ref).map_or(0, |item| item.count))
                .map(|run| {
                    self.item_at(bytes, data_runs_ref, run)
                        .map(|data| data[..4].try_into().unwrap())
                })
                .collect::<Result<_, _>>()?;
        let section_count = self
            .items
            .get(sections_ref)
            .ok_or("missing compressed section item")?
            .count;
        let primitives_count = self
            .items
            .get(primitives_ref)
            .ok_or("missing compressed primitive item")?
            .count;
        let shared_index_count = self
            .items
            .get(shared_index_ref)
            .ok_or("missing shared index item")?
            .count;
        let packed_count = self
            .items
            .get(packed_ref)
            .ok_or("missing packed vertex item")?
            .count;
        let shared_count = self
            .items
            .get(shared_ref)
            .ok_or("missing shared vertex item")?
            .count;
        let (domain_min, domain_max) = self.compressed_domain(tree, tree_type)?;
        for section_number in 0..section_count {
            let section = self.item_at(bytes, sections_ref, section_number)?;
            let base = [
                float(section, 0x30)?,
                float(section, 0x34)?,
                float(section, 0x38)?,
            ];
            let scale = [
                float(section, 0x3c)?,
                float(section, 0x40)?,
                float(section, 0x44)?,
            ];
            let first_packed = le32(section, 0x48)? as usize;
            let shared_header = le32(section, 0x4c)? as usize;
            let primitive_header = le32(section, 0x50)? as usize;
            let num_packed = section[0x58] as usize;
            let num_shared = section[0x59] as usize;
            let first_shared = shared_header >> 8;
            let first_primitive = primitive_header >> 8;
            let num_primitives = primitive_header & 0xff;
            let runs_header = le32(section, 0x54)? as usize;
            let runs = data_runs
                .get(runs_header >> 8..(runs_header >> 8) + (runs_header & 0xff))
                .ok_or_else(|| {
                    format!("compressed shape {shape_index} section {section_number}: data runs out of range")
                })?;
            let primitive_tags = |local: usize| -> Result<ShapeTagPath, String> {
                let mut tags = shape_tags;
                tags[depth] = section_primitive_tag(runs, local).ok_or_else(|| {
                    format!("compressed shape {shape_index} section {section_number}: primitive {local} has no data run")
                })?;
                Ok(tags)
            };
            if first_packed + num_packed > packed_count
                || first_shared + num_shared > shared_index_count
                || first_primitive + num_primitives > primitives_count
            {
                return Err(format!(
                    "compressed shape {shape_index} section {section_number}: array range invalid"
                ));
            }
            // FUN_14127f590 dispatches each primitive independently. Convex
            // headers can coexist with ordinary triangles in one section.
            let domain = [
                [
                    float(section, 0x10)?,
                    float(section, 0x14)?,
                    float(section, 0x18)?,
                    float(section, 0x1c)?,
                ],
                [
                    float(section, 0x20)?,
                    float(section, 0x24)?,
                    float(section, 0x28)?,
                    float(section, 0x2c)?,
                ],
            ];
            let nodes_ref = le32(section, 0)? as usize;
            let node_count = self.items.get(nodes_ref).map_or(0, |item| item.count);
            let nodes: Vec<[u8; 4]> = (0..node_count)
                .map(|node| {
                    self.item_at(bytes, nodes_ref, node)
                        .map(|data| data[..4].try_into().unwrap())
                })
                .collect::<Result<_, _>>()?;
            for number in first_primitive..first_primitive + num_primitives {
                let primitive = self.item_at(bytes, primitives_ref, number)?;
                if compressed_primitive_kind(primitive) == CompressedPrimitiveKind::Convex {
                    let local_index = primitive[0] as usize;
                    // Native indexes individual u16 words; authored headers
                    // may include a third property word, so odd indices are valid.
                    if local_index < num_packed {
                        return Err(format!("compressed shape {shape_index} section {section_number}: invalid convex header index"));
                    }
                    let header_index = first_shared + local_index - num_packed;
                    if header_index + 1 >= first_shared + num_shared {
                        return Err(format!("compressed shape {shape_index} section {section_number}: convex leaf header out of range"));
                    }
                    let header = u16::from_le_bytes(
                        self.item_at(bytes, shared_index_ref, header_index)?[..2]
                            .try_into()
                            .unwrap(),
                    );
                    let start = u16::from_le_bytes(
                        self.item_at(bytes, shared_index_ref, header_index + 1)?[..2]
                            .try_into()
                            .unwrap(),
                    ) as usize;
                    let count = ((header >> 8) as usize).min(0x70);
                    let codec = (header >> 4) & 3;
                    let words = match codec {
                        0 => count,
                        1 => (count + 1) / 2,
                        2 => (count + 3) / 4,
                        _ => 0,
                    };
                    if header & 0xf != 2 || codec > 2 || count < 3 || start + words > shared_count {
                        return Err(format!("compressed shape {shape_index} section {section_number}: unsupported convex leaf header {header:#x}, start={start}"));
                    }
                    // The primitive's second byte selects its BVH node, not
                    // the section-wide domain (FUN_14127f030).
                    let [min, max] =
                        compressed_tree_node_bounds(domain, &nodes, primitive[1] as usize)?;
                    let low = if codec == 0 {
                        domain_min
                    } else {
                        [min[0], min[1], min[2]]
                    };
                    let high = if codec == 0 {
                        domain_max
                    } else {
                        [max[0], max[1], max[2]]
                    };
                    let divisor = match codec {
                        0 => [1.0 / 2097151.0, 1.0 / 2097151.0, 1.0 / 4194303.0],
                        1 => [
                            f32::from_bits(0x3a00_1002),
                            f32::from_bits(0x3a00_1002),
                            f32::from_bits(0x3a80_2008),
                        ],
                        2 => [
                            f32::from_bits(0x3d04_2108),
                            f32::from_bits(0x3d04_2108),
                            f32::from_bits(0x3c82_0821),
                        ],
                        _ => unreachable!(),
                    };
                    let scale: [f32; 3] =
                        std::array::from_fn(|axis| (high[axis] - low[axis]) * divisor[axis]);
                    let mut hull = Vec::with_capacity(count);
                    for vertex in 0..count {
                        let quantized = match codec {
                            0 => {
                                let value = u64::from_le_bytes(
                                    self.item_at(bytes, shared_ref, start + vertex)?[..8]
                                        .try_into()
                                        .unwrap(),
                                );
                                [
                                    (value & 0x1f_ffff) as f32,
                                    ((value >> 21) & 0x1f_ffff) as f32,
                                    (value >> 42) as f32,
                                ]
                            }
                            1 => {
                                // Native swaps each u32 pair, including odd tails.
                                let word = start * 2 + (vertex ^ 1);
                                let value = le32(
                                    self.item_at(bytes, shared_ref, word / 2)?,
                                    (word & 1) * 4,
                                )?;
                                [
                                    (value & 0x7ff) as f32,
                                    ((value >> 11) & 0x7ff) as f32,
                                    (value >> 22) as f32,
                                ]
                            }
                            2 => {
                                // Native reverses each four-u16 group.
                                let word = start * 4 + (vertex ^ 3);
                                let data = self.item_at(bytes, shared_ref, word / 4)?;
                                let value = u16::from_le_bytes(
                                    data[(word & 3) * 2..(word & 3) * 2 + 2].try_into().unwrap(),
                                );
                                [
                                    (value & 31) as f32,
                                    ((value >> 5) & 31) as f32,
                                    (value >> 10) as f32,
                                ]
                            }
                            _ => unreachable!(),
                        };
                        let local =
                            std::array::from_fn(|axis| quantized[axis] * scale[axis] + low[axis]);
                        hull.push(transform.apply(local));
                    }
                    triangulate_convex_vertices(
                        &hull,
                        shape_index,
                        primitive_tags(number - first_primitive)?,
                        &mut output.triangles,
                    )?;
                    output.convex.push(ConvexCollider {
                        vertices: hull,
                        source_shape_item: shape_index,
                    });
                }
            }
            // Decode ordinary corners only when referenced. Shared-index
            // entries belonging to convex headers are not vertex indices.
            let corner = |index: u8| -> Result<[f32; 3], String> {
                let number = index as usize;
                if number < num_packed {
                    let value = le32(self.item_at(bytes, packed_ref, first_packed + number)?, 0)?;
                    return Ok([
                        base[0] + (value & 0x7ff) as f32 * scale[0],
                        base[1] + ((value >> 11) & 0x7ff) as f32 * scale[1],
                        base[2] + (value >> 22) as f32 * scale[2],
                    ]);
                }
                if number >= num_packed + num_shared {
                    return Err(format!("compressed shape {shape_index} section {section_number}: vertex index out of range"));
                }
                let shared_index = u16::from_le_bytes(
                    self.item_at(bytes, shared_index_ref, first_shared + number - num_packed)?[..2]
                        .try_into()
                        .unwrap(),
                ) as usize;
                if shared_index >= shared_count {
                    return Err(format!("compressed shape {shape_index} section {section_number}: shared vertex index {shared_index} >= {shared_count}"));
                }
                let value = u64::from_le_bytes(
                    self.item_at(bytes, shared_ref, shared_index)?[..8]
                        .try_into()
                        .unwrap(),
                );
                Ok([
                    domain_min[0]
                        + (value & 0x1f_ffff) as f32 / 0x1f_ffff as f32
                            * (domain_max[0] - domain_min[0]),
                    domain_min[1]
                        + ((value >> 21) & 0x1f_ffff) as f32 / 0x1f_ffff as f32
                            * (domain_max[1] - domain_min[1]),
                    domain_min[2]
                        + ((value >> 42) & 0x3f_ffff) as f32 / 0x3f_ffff as f32
                            * (domain_max[2] - domain_min[2]),
                ])
            };
            for number in 0..num_primitives {
                let primitive = self.item_at(bytes, primitives_ref, first_primitive + number)?;
                if !primitive_has_surface(primitive) {
                    continue;
                }
                let corner = |at: usize| corner(primitive[at]);
                let leaf_tags = primitive_tags(number)?;
                if distinct3(primitive[0], primitive[1], primitive[2]) {
                    let [a, b, c] = [corner(0)?, corner(1)?, corner(2)?];
                    output.triangles.push(MeshTriangle {
                        vertices: [transform.apply(a), transform.apply(b), transform.apply(c)],
                        source_shape_item: shape_index,
                        shape_tags: leaf_tags,
                        material: None,
                    });
                }
                if distinct3(primitive[0], primitive[2], primitive[3]) {
                    let [a, c, d] = [corner(0)?, corner(2)?, corner(3)?];
                    output.triangles.push(MeshTriangle {
                        vertices: [transform.apply(a), transform.apply(c), transform.apply(d)],
                        source_shape_item: shape_index,
                        shape_tags: leaf_tags,
                        material: None,
                    });
                }
            }
        }
        Ok(())
    }
}

/// FUN_14124e030: a compressed-mesh primitive's shape tag is the `value` of
/// the section data run (hkcdStaticMeshTreeBase::PrimitiveDataRunBase:
/// value u16, index u8, count u8) whose `[index, index + count)` holds the
/// primitive's section-local index. The game searches the section's runs
/// (binary above four, then linearly) and takes the first match.
fn section_primitive_tag(runs: &[[u8; 4]], local: usize) -> Option<u16> {
    runs.iter()
        .find(|run| (run[2] as usize..run[2] as usize + run[3] as usize).contains(&local))
        .map(|run| u16::from_le_bytes([run[0], run[1]]))
}

fn distinct3(a: u8, b: u8, c: u8) -> bool {
    a != b && b != c && a != c
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompressedPrimitiveKind {
    Empty,
    Triangle,
    Quad,
    Convex,
}

fn compressed_primitive_kind(primitive: &[u8]) -> CompressedPrimitiveKind {
    if primitive == [0xde, 0xad, 0xde, 0xad] {
        CompressedPrimitiveKind::Empty
    } else if primitive[1] == primitive[3] {
        CompressedPrimitiveKind::Convex
    } else if primitive[2] == primitive[3] {
        CompressedPrimitiveKind::Triangle
    } else {
        CompressedPrimitiveKind::Quad
    }
}

fn primitive_has_surface(primitive: &[u8]) -> bool {
    matches!(
        compressed_primitive_kind(primitive),
        CompressedPrimitiveKind::Triangle | CompressedPrimitiveKind::Quad
    )
}

// FUN_140c253a0: Codec3Axis4 child bounds; fourth byte's low nibble
// is a tree link, not a W-axis upper shrink. Constants read from running DH2.
fn compressed_tree_child_bounds(parent: [[f32; 4]; 2], node: [u8; 4]) -> [[f32; 4]; 2] {
    let scale: [f32; 4] = std::array::from_fn(|axis| {
        (parent[1][axis] - parent[0][axis]) * f32::from_bits(0x3b90_fdbc)
    });
    [
        std::array::from_fn(|axis| {
            let q = (node[axis] >> 4) as f32;
            q * q * scale[axis] + parent[0][axis]
        }),
        std::array::from_fn(|axis| {
            let q = if axis == 3 {
                0.0
            } else {
                (node[axis] & 15) as f32
            };
            parent[1][axis] - q * q * scale[axis]
        }),
    ]
}

// FUN_14127f030: follow the packed child offsets to a requested BVH node.
fn compressed_tree_node_bounds(
    mut bounds: [[f32; 4]; 2],
    nodes: &[[u8; 4]],
    target: usize,
) -> Result<[[f32; 4]; 2], String> {
    let mut current = 0;
    while current != target {
        let node = nodes
            .get(current)
            .ok_or("compressed tree node out of range")?;
        let right = current + (node[3] & 0xfe) as usize;
        let next = if right <= target { right } else { current + 1 };
        if next <= current || next > target {
            return Err("compressed tree has invalid child offset".into());
        }
        bounds = compressed_tree_child_bounds(
            bounds,
            *nodes
                .get(next)
                .ok_or("compressed tree child out of range")?,
        );
        current = next;
    }
    Ok(bounds)
}

fn triangulate_convex_vertices(
    vertices: &[[f32; 3]],
    source_shape_item: usize,
    shape_tags: ShapeTagPath,
    triangles: &mut Vec<MeshTriangle>,
) -> Result<(), String> {
    let sub = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| a[i] - b[i]) };
    let dot = |a: [f32; 3], b: [f32; 3]| -> f32 { (0..3).map(|i| a[i] * b[i]).sum() };
    let cross = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let extent = (0..3)
        .map(|i| {
            let low = vertices.iter().map(|p| p[i]).fold(f32::INFINITY, f32::min);
            let high = vertices
                .iter()
                .map(|p| p[i])
                .fold(f32::NEG_INFINITY, f32::max);
            high - low
        })
        .fold(0.0f32, f32::max);
    let epsilon = (extent * 0.00001).max(0.000001);
    let first = (0..vertices.len())
        .min_by(|&a, &b| vertices[a][0].total_cmp(&vertices[b][0]))
        .unwrap();
    let second = (0..vertices.len())
        .max_by(|&a, &b| vertices[a][0].total_cmp(&vertices[b][0]))
        .unwrap();
    let line = sub(vertices[second], vertices[first]);
    let third = (0..vertices.len())
        .max_by(|&a, &b| {
            let distance = |i| {
                dot(
                    cross(line, sub(vertices[i], vertices[first])),
                    cross(line, sub(vertices[i], vertices[first])),
                )
            };
            distance(a).total_cmp(&distance(b))
        })
        .unwrap();
    let plane = cross(line, sub(vertices[third], vertices[first]));
    if dot(plane, plane).sqrt() < epsilon * dot(line, line).sqrt() {
        return Err(format!(
            "compressed shape {source_shape_item}: convex leaf is a line"
        ));
    }
    let fourth = (0..vertices.len())
        .max_by(|&a, &b| {
            dot(plane, sub(vertices[a], vertices[first]))
                .abs()
                .total_cmp(&dot(plane, sub(vertices[b], vertices[first])).abs())
        })
        .unwrap();
    let seed = [first, second, third, fourth];
    if dot(plane, sub(vertices[fourth], vertices[first])).abs() < epsilon * dot(plane, plane).sqrt()
    {
        let center: [f32; 3] = std::array::from_fn(|i| {
            vertices.iter().map(|p| p[i]).sum::<f32>() / vertices.len() as f32
        });
        let side = cross(plane, line);
        let mut ordered: Vec<usize> = (0..vertices.len()).collect();
        ordered.sort_by(|&a, &b| {
            let angle = |i| {
                let v = sub(vertices[i], center);
                dot(v, side).atan2(dot(v, line))
            };
            angle(a).total_cmp(&angle(b))
        });
        for i in 1..ordered.len() - 1 {
            triangles.push(MeshTriangle {
                vertices: [
                    vertices[ordered[0]],
                    vertices[ordered[i]],
                    vertices[ordered[i + 1]],
                ],
                source_shape_item,
                shape_tags,
                material: None,
            });
        }
        return Ok(());
    }
    let inside: [f32; 3] =
        std::array::from_fn(|i| seed.iter().map(|&v| vertices[v][i]).sum::<f32>() / 4.0);
    let orient = |mut face: [usize; 3]| -> [usize; 3] {
        let normal = cross(
            sub(vertices[face[1]], vertices[face[0]]),
            sub(vertices[face[2]], vertices[face[0]]),
        );
        if dot(normal, sub(inside, vertices[face[0]])) > 0.0 {
            face.swap(1, 2);
        }
        face
    };
    let mut faces = vec![
        orient([first, second, third]),
        orient([first, fourth, second]),
        orient([first, third, fourth]),
        orient([second, fourth, third]),
    ];
    for point in 0..vertices.len() {
        if seed.contains(&point) {
            continue;
        }
        let visible = |face: &[usize; 3]| {
            let normal = cross(
                sub(vertices[face[1]], vertices[face[0]]),
                sub(vertices[face[2]], vertices[face[0]]),
            );
            dot(normal, sub(vertices[point], vertices[face[0]]))
                > epsilon * dot(normal, normal).sqrt()
        };
        let mut horizon = Vec::<(usize, usize)>::new();
        for face in faces.iter().filter(|face| visible(face)) {
            for edge in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
                if let Some(index) = horizon
                    .iter()
                    .position(|&(a, b)| a == edge.1 && b == edge.0)
                {
                    horizon.swap_remove(index);
                } else {
                    horizon.push(edge);
                }
            }
        }
        if horizon.is_empty() {
            continue;
        }
        faces.retain(|face| !visible(face));
        for (a, b) in horizon {
            faces.push(orient([a, b, point]));
        }
    }
    for face in faces {
        triangles.push(MeshTriangle {
            vertices: face.map(|i| vertices[i]),
            source_shape_item,
            shape_tags,
            material: None,
        });
    }
    Ok(())
}

fn float(bytes: &[u8], at: usize) -> Result<f32, String> {
    Ok(f32::from_le_bytes(slice(bytes, at, 4)?.try_into().unwrap()))
}

fn convex_quad_order(vertices: &[[f32; 3]]) -> Result<[usize; 4], String> {
    let difference =
        |a: [f32; 3], b: [f32; 3]| -> [f32; 3] { std::array::from_fn(|axis| a[axis] - b[axis]) };
    let dot = |a: [f32; 3], b: [f32; 3]| -> f32 { (0..3).map(|axis| a[axis] * b[axis]).sum() };
    let cross = |a: [f32; 3], b: [f32; 3]| -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let u = difference(vertices[1], vertices[0]);
    let normal = cross(u, difference(vertices[2], vertices[0]));
    if dot(normal, normal) < 1e-12
        || vertices
            .iter()
            .any(|&point| dot(normal, difference(point, vertices[0])).abs() > 0.001)
    {
        return Err("generic four-vertex convex shape is not a planar quad".into());
    }
    let v = cross(normal, u);
    let center: [f32; 3] =
        std::array::from_fn(|axis| vertices.iter().map(|vertex| vertex[axis]).sum::<f32>() / 4.0);
    let mut order = [0, 1, 2, 3];
    order.sort_by(|&a, &b| {
        let angle = |index| {
            let relative = difference(vertices[index], center);
            dot(relative, v).atan2(dot(relative, u))
        };
        angle(a).total_cmp(&angle(b))
    });
    Ok(order)
}

#[derive(Clone, Copy)]
struct Affine {
    columns: [[f32; 3]; 3],
    translation: [f32; 3],
}

impl Affine {
    fn translated(translation: [f32; 3]) -> Self {
        Self {
            columns: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            translation,
        }
    }
    fn from_havok_instance(bytes: &[u8]) -> Result<Self, String> {
        let mut columns = [[0.0; 3]; 3];
        for (column, vector) in columns.iter_mut().enumerate() {
            for (axis, value) in vector.iter_mut().enumerate() {
                *value = float(bytes, column * 16 + axis * 4)?;
            }
        }
        let translation = [
            float(bytes, 0x30)?,
            float(bytes, 0x34)?,
            float(bytes, 0x38)?,
        ];
        let scale = [
            float(bytes, 0x40)?,
            float(bytes, 0x44)?,
            float(bytes, 0x48)?,
        ];
        for (column, vector) in columns.iter_mut().enumerate() {
            for value in vector {
                *value *= scale[column];
            }
        }
        Ok(Self {
            columns,
            translation,
        })
    }
    fn apply(self, point: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|axis| {
            self.translation[axis]
                + (0..3)
                    .map(|column| self.columns[column][axis] * point[column])
                    .sum::<f32>()
        })
    }
    fn compose(self, other: Self) -> Self {
        let columns = std::array::from_fn(|column| {
            std::array::from_fn(|axis| {
                (0..3)
                    .map(|inner| self.columns[inner][axis] * other.columns[column][inner])
                    .sum()
            })
        });
        Self {
            columns,
            translation: self.apply(other.translation),
        }
    }
}

/// FUN_140cc3ba0 writes the original per-body byte classifier output.
/// The byte codes remain raw: their higher-level meaning is not established.
pub fn classify_body_rows(body_ids: &[u32], rows: &[u8], output: &mut [u8]) -> Result<(), String> {
    const ROW_STRIDE: usize = 0xb0;
    if output.len() < body_ids.len() {
        return Err("body-row output is shorter than the input ID list".into());
    }
    for (slot, body_id) in body_ids.iter().enumerate() {
        let row_start = ((*body_id & 0x00ff_ffff) as usize)
            .checked_mul(ROW_STRIDE)
            .ok_or("body-row offset overflow")?;
        let row_end = row_start
            .checked_add(0x49)
            .ok_or("body-row extent overflow")?;
        let row = rows
            .get(row_start..row_end)
            .ok_or_else(|| format!("body ID {body_id:#x} exceeds the row table"))?;
        let flags = u32::from_le_bytes(row[0x44..0x48].try_into().unwrap());
        let row_byte = row[0x48];
        let selected = if flags & 0x100 != 0 {
            1
        } else if flags & 8 != 0 {
            0
        } else if flags & 1 == 0 {
            if row_byte & 0x91 == 0 {
                2
            } else {
                0
            }
        } else if row_byte & 0x91 == 0 {
            4
        } else {
            3
        };
        output[slot] = selected;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shape_tag_codec_follows_arkshapetagcodeclocal_decode() {
        let path = |tags: &[u16]| {
            let mut path = NO_SHAPE_TAGS;
            path[..tags.len()].copy_from_slice(tags);
            path
        };
        let map: Vec<u32> = (0..16).map(|index| 100 + index).collect();
        // A relative leaf tag indexes the world table from the body id.
        assert_eq!(shape_tag_material(0, &path(&[0x5]), &map), Some(105));
        assert_eq!(shape_tag_material(2, &path(&[0x5]), &map), Some(107));
        // An absolute instance tag sets the base for its relative leaf tags.
        assert_eq!(
            shape_tag_material(0, &path(&[0x8007, 0x1]), &map),
            Some(108)
        );
        // Invalid tags are skipped anywhere on the path.
        assert_eq!(
            shape_tag_material(0, &path(&[NO_SHAPE_TAG, 0x3]), &map),
            Some(103)
        );
        // A raw id result names a runtime library entry; a relative tag after a
        // table hit adds a runtime library id. Neither is statically known.
        assert_eq!(shape_tag_material(0, &NO_SHAPE_TAGS, &map), None);
        assert_eq!(shape_tag_material(0, &path(&[0x8003]), &map), None);
        assert_eq!(shape_tag_material(0, &path(&[0x1, 0x1]), &map), None);
        // Out-of-table indices stay unresolved.
        assert_eq!(shape_tag_material(0, &path(&[0x20]), &map), None);
    }

    #[test]
    fn compressed_primitive_tag_comes_from_its_section_data_run() {
        // value u16, first primitive, count.
        let runs = [[0x07, 0x00, 0, 3], [0x2f, 0x00, 3, 2], [0x01, 0x80, 5, 1]];
        assert_eq!(section_primitive_tag(&runs, 0), Some(7));
        assert_eq!(section_primitive_tag(&runs, 2), Some(7));
        assert_eq!(section_primitive_tag(&runs, 4), Some(0x2f));
        assert_eq!(section_primitive_tag(&runs, 5), Some(0x8001));
        assert_eq!(section_primitive_tag(&runs, 6), None);
    }

    #[test]
    fn body_row_classifier_preserves_branch_order_and_id_mask() {
        const STRIDE: usize = 0xb0;
        fn set_row(rows: &mut [u8], id: usize, flags: u32, row_byte: u8) {
            let start = id * STRIDE;
            rows[start + 0x44..start + 0x48].copy_from_slice(&flags.to_le_bytes());
            rows[start + 0x48] = row_byte;
        }
        let mut rows = vec![0; 6 * STRIDE];
        set_row(&mut rows, 0, 0x100, 0x91); // bit 8 wins over later tests
        set_row(&mut rows, 1, 8, 0x91); // bit 3 suppresses the remaining tests
        set_row(&mut rows, 2, 0, 0);
        set_row(&mut rows, 3, 1, 0);
        set_row(&mut rows, 4, 0, 1); // byte mask 0x91 set
        set_row(&mut rows, 5, 1, 1);

        let ids = [0, 1, 2, 3, 4, 5, 0x0100_0001];
        let mut output = [0xaa; 8];
        classify_body_rows(&ids, &rows, &mut output).unwrap();
        assert_eq!(output, [1, 0, 2, 4, 0, 3, 0, 0xaa]);
    }

    #[test]
    fn body_row_classifier_checks_accessible_input_and_preserves_empty_call() {
        let mut output = [0x77];
        classify_body_rows(&[], &[], &mut output).unwrap();
        assert_eq!(output, [0x77]);
        assert!(classify_body_rows(&[0], &[0; 0x49], &mut []).is_err());
        assert!(classify_body_rows(&[1], &[0; 0xb0], &mut [0]).is_err());
    }

    #[test]
    fn rejects_short_resource() {
        assert!(parse(&[0u8; 3]).is_err());
    }

    #[test]
    fn physics_shape_kinds_match_suffix_order() {
        assert_eq!(
            replace_shape_extension("folder.with.dot/fence.blueprint", "hkt"),
            "folder.with.dot/fence.hkt"
        );
        assert_eq!(
            replace_shape_extension("folder.with.dot/fence", ".hkt"),
            "folder.with.dot/fence.hkt"
        );
        assert_eq!(
            replace_shape_extension("folder\\with.dot\\fence.lwo", "cvx"),
            "folder\\with.dot\\fence.cvx"
        );
        assert_eq!(
            hkt_shape_name("models/fence.lwo", false),
            "models/fence.hkt"
        );
        assert_eq!(
            hkt_shape_name("models/fence.lwo", true),
            "models/fence_hd.hkt"
        );
        assert_eq!(
            physics_shape_asset_path("models/props/fence.blueprint.hkt._100_100_100.10"),
            "generated/models/props/fence.blueprint.hkt._100_100_100.10.hkx"
        );
        assert_eq!(
            physics_shape_asset_path("models/fence.hkt._100_100_100.10\0ignored"),
            "generated/models/fence.hkt._100_100_100.10.hkx"
        );
        for (name, expected) in [
            ("hkt", 0),
            ("CVX", 1),
            ("MsH", 2),
            ("box", 3),
            (".hkt", 4),
            ("hktx", 4),
            ("", 4),
            ("hkt\0ignored", 0),
        ] {
            assert_eq!(physics_shape_kind(name), expected);
        }
    }

    #[test]
    fn physics_shape_comparison_and_kinds_match() {
        let cases = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/physics_shape_kind_cases.txt"
        ))
        .expect("missing native physics shape comparison/kind fixture");
        let decode = |text: &str| -> Vec<u8> {
            if text == "-" {
                return Vec::new();
            }
            text.as_bytes()
                .chunks_exact(2)
                .map(|hex| u8::from_str_radix(std::str::from_utf8(hex).unwrap(), 16).unwrap())
                .collect()
        };
        let mut count = 0;
        for line in cases.lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let left = decode(fields[1]);
            if fields[0] == "C" {
                let right = decode(fields[2]);
                assert_eq!(
                    ascii_cstr_compare(&left, &right),
                    fields[3].parse::<i32>().unwrap(),
                    "{line}"
                );
            } else {
                assert_eq!(
                    physics_shape_kind(std::str::from_utf8(&left).unwrap()),
                    fields[2].parse::<u8>().unwrap(),
                    "{line}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 1043);
    }

    #[test]
    fn physics_shape_recipe_matches_non_hd_builder() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/physics_shape_key_cases.txt"
        ))
        .lines()
        {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let values: Vec<u32> = fields[..5]
                .iter()
                .map(|value| value.parse().unwrap())
                .collect();
            let scale = std::array::from_fn(|axis| f32::from_bits(values[axis + 1]));
            let expected: Vec<u8> = fields[5]
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            assert_eq!(
                physics_shape_key(
                    "models/fence.blueprint",
                    values[0] as u8,
                    scale,
                    f32::from_bits(values[4]),
                    false
                ),
                Some(String::from_utf8(expected).unwrap()),
                "ordinary physics recipe {count}"
            );
            count += 1;
        }
        assert_eq!(count, 1088);
        assert_eq!(
            physics_shape_key("a\0ignored", 0, [-0.015, 0.0001, -0.0001], 0.01, false),
            Some("a.hkt._-1_1_-1.10".into())
        );
        assert_eq!(physics_shape_key("a", 5, [1.0; 3], 0.01, false), None);
        assert_eq!(physics_shape_key("a", 0, [f32::NAN; 3], 0.01, false), None);
        assert_eq!(physics_shape_key("a", 0, [1.0; 3], f32::NAN, false), None);
    }
    #[test]
    fn physics_hd_recipe_matches_builder() {
        let decode = |text: &str| -> String {
            if text == "-" {
                return String::new();
            }
            String::from_utf8(
                text.as_bytes()
                    .chunks_exact(2)
                    .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                    .collect(),
            )
            .unwrap()
        };
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/physics_shape_hd_cases.txt"
        ))
        .lines()
        {
            let fields: Vec<_> = line.split_whitespace().collect();
            let values: Vec<u32> = fields[..5]
                .iter()
                .map(|value| value.parse().unwrap())
                .collect();
            let scale = std::array::from_fn(|axis| f32::from_bits(values[axis + 1]));
            assert_eq!(
                physics_shape_key(
                    &decode(fields[5]),
                    values[0] as u8,
                    scale,
                    f32::from_bits(values[4]),
                    true
                ),
                Some(decode(fields[6])),
                "{line}"
            );
            count += 1;
        }
        assert_eq!(count, 1143);
        assert_eq!(
            physics_shape_key("models/fence.blueprint\0ignored", 0, [1.0; 3], 0.01, true),
            Some("models/fence_hd.blueprint.hkt._100_100_100.10".into())
        );
    }
    #[test]
    #[ignore = "requires local installed-game fixture artifacts/railing-blueprint-root.hkx"]
    fn standalone_compressed_blueprint_layout_and_odd_headers() {
        let bytes = std::fs::read("artifacts/railing-blueprint-root.hkx")
            .expect("missing required railing-blueprint-root.hkx fixture");
        let world = parse_hkx(&bytes).unwrap();
        assert_eq!(
            world
                .field_offset(world.items[1].type_index, "data")
                .unwrap(),
            64
        );
        assert_eq!(
            world
                .field_offset(world.items[2].type_index, "meshTree")
                .unwrap(),
            16
        );
        let geometry = world.standalone_shapes(&bytes).unwrap();
        assert!(
            geometry.unknown_shapes.is_empty(),
            "{:?}",
            geometry.unknown_shapes
        );
        assert!(
            geometry.compressed_decode_failures.is_empty(),
            "{:?}",
            geometry.compressed_decode_failures
        );
        assert_eq!(geometry.compressed.len(), 1);
        assert_eq!(geometry.convex.len(), 26);
        assert_eq!(geometry.triangles.len(), 312);
        assert!(geometry.convex.iter().all(|hull| hull.vertices.len() == 8));
        let bounds = &geometry.compressed[0];
        for triangle in &geometry.triangles {
            for point in triangle.vertices {
                for axis in 0..3 {
                    assert!(point[axis].is_finite());
                    assert!(point[axis] >= bounds.bounds_min[axis] - 0.0001);
                    assert!(point[axis] <= bounds.bounds_max[axis] + 0.0001);
                }
            }
        }
    }
    #[test]
    fn rigid_body_basis_matches_conversions() {
        let mut matrices = 0;
        let mut quaternions = 0;
        let same = |actual: &[f32], expected: &[u32], line: &str| {
            for (actual, expected) in actual.iter().zip(expected) {
                assert_eq!(actual.to_bits(), *expected, "{line}");
            }
        };
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/rigid_body_basis_cases.txt"
        ))
        .lines()
        {
            let mut fields = line.split_whitespace();
            let kind = fields.next().unwrap();
            let words: Vec<u32> = fields.map(|value| value.parse().unwrap()).collect();
            if kind == "M" {
                let basis = std::array::from_fn(|column| {
                    std::array::from_fn(|lane| f32::from_bits(words[column * 4 + lane]))
                });
                same(&basis_quaternion(basis), &words[12..16], line);
                let actual = rigid_body_basis(basis);
                for column in 0..3 {
                    same(
                        &actual[column],
                        &words[16 + column * 4..20 + column * 4],
                        line,
                    );
                }
                matrices += 1;
            } else {
                assert_eq!(kind, "Q");
                let actual =
                    quaternion_basis(std::array::from_fn(|lane| f32::from_bits(words[lane])));
                for column in 0..3 {
                    same(
                        &actual[column],
                        &words[4 + column * 4..8 + column * 4],
                        line,
                    );
                }
                quaternions += 1;
            }
        }
        assert_eq!((matrices, quaternions), (1076, 263));
    }

    #[test]
    fn reads_scaled_standalone_havok_alias() {
        let target = b"models/test/crate.lwo.hkt._100_100_100.10";
        let mut bytes = vec![0, 0, 0, 0, 1];
        bytes.extend_from_slice(&0.8f32.to_le_bytes());
        bytes.extend_from_slice(&(target.len() as u32).to_le_bytes());
        bytes.extend_from_slice(target);
        assert_eq!(
            standalone_hkx_alias(&bytes),
            Some((
                "generated/models/test/crate.lwo.hkt._100_100_100.10.hkx".into(),
                0.8
            ))
        );
    }
    #[test]
    fn distinguishes_surface_quads_from_point_and_line_proxies() {
        assert!(!primitive_has_surface(&[6, 7, 7, 7]));
        assert!(!primitive_has_surface(&[8, 8, 8, 8]));
        assert!(primitive_has_surface(&[6, 7, 8, 8]));
        assert!(primitive_has_surface(&[6, 6, 7, 8]));
    }
    #[test]
    fn compressed_primitive_header_dispatch_matches() {
        assert_eq!(
            compressed_primitive_kind(&[0xde, 0xad, 0xde, 0xad]),
            CompressedPrimitiveKind::Empty
        );
        assert_eq!(
            compressed_primitive_kind(&[32, 6, 11, 6]),
            CompressedPrimitiveKind::Convex
        );
        assert_eq!(
            compressed_primitive_kind(&[0, 1, 2, 2]),
            CompressedPrimitiveKind::Triangle
        );
        assert_eq!(
            compressed_primitive_kind(&[0, 1, 2, 3]),
            CompressedPrimitiveKind::Quad
        );
    }
    #[test]
    fn compressed_tree_bounds_match_installed_executable() {
        let mut cases = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/compressed_tree_cases.txt"
        ))
        .lines()
        {
            let fields: Vec<u32> = line
                .split_whitespace()
                .map(|field| field.parse().unwrap())
                .collect();
            let count = fields[1] as usize;
            let domain = std::array::from_fn(|side| {
                std::array::from_fn(|axis| f32::from_bits(fields[3 + side * 4 + axis]))
            });
            let nodes: Vec<[u8; 4]> = (0..count)
                .map(|node| std::array::from_fn(|axis| fields[11 + node * 4 + axis] as u8))
                .collect();
            let result = if fields[0] == 0 {
                compressed_tree_child_bounds(domain, nodes[0])
            } else {
                compressed_tree_node_bounds(domain, &nodes, fields[2] as usize).unwrap()
            };
            let expected = &fields[11 + count * 4..];
            assert_eq!(
                result
                    .concat()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>(),
                expected,
                "compressed bounds case{cases}"
            );
            cases += 1;
        }
        assert_eq!(cases, 1024);
    }
    #[test]
    fn packed_integers_cover_all_widths() {
        let bytes = [
            0x7f, 0x81, 0x02, 0xc1, 0x02, 0x03, 0xe1, 0x02, 0x03, 0x04, 0xe8, 0x7f, 0xff, 0xff,
            0xff,
        ];
        let mut at = 0;
        assert_eq!(packed(&bytes, &mut at, bytes.len()).unwrap(), 127);
        assert_eq!(packed(&bytes, &mut at, bytes.len()).unwrap(), 258);
        assert_eq!(packed(&bytes, &mut at, bytes.len()).unwrap(), 0x10203);
        assert_eq!(packed(&bytes, &mut at, bytes.len()).unwrap(), 0x1020304);
        assert_eq!(packed(&bytes, &mut at, bytes.len()).unwrap(), 0x7fff_ffff);
    }
    #[test]
    fn generic_planar_convex_quad_is_ordered_without_crossing() {
        let vertices = [
            [0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 2.0, 1.0],
        ];
        let order = convex_quad_order(&vertices).unwrap();
        let area = |indices: [usize; 3]| {
            let [a, b, c] = indices.map(|index| vertices[index]);
            ((b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1])).abs() * 0.5
        };
        assert!(
            (area([order[0], order[1], order[2]]) + area([order[0], order[2], order[3]]) - 2.0)
                .abs()
                < 1e-5
        );
        assert!(convex_quad_order(&[[0.0; 3]; 4]).is_err());
    }
    #[test]
    #[ignore = "audit; requires installed Dust District archives"]
    fn installed_cable_physics_system_layout() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        let path = "generated/physsystem/models/environment/gameplay/speaker_01/cables_speaker/cable_01_rgdl.hkx";
        let bytes = catalog.read(path).unwrap().unwrap();
        let world = parse_embedded_havok(&bytes).unwrap();
        let mut seen = std::collections::BTreeSet::new();
        fn dump(world: &PhysWorld, index: usize, depth: usize, seen: &mut std::collections::BTreeSet<usize>) {
            if depth > 6 || !seen.insert(index) {
                return;
            }
            let ty = &world.types[index];
            println!("{}TYPE {} size={} parent={} sub={:#x} ptr={}", "  ".repeat(depth), ty.name, ty.byte_size,
                world.types.get(ty.parent).map(|p| p.name.as_str()).unwrap_or("-"), ty.subtype, ty.pointer);
            for field in &ty.fields {
                println!("{}  +{:#x} {} : {}", "  ".repeat(depth), field.offset, field.name, world.types[field.type_index].name);
                dump(world, field.type_index, depth + 2, seen);
            }
            if ty.parent != 0 && ty.parent < world.types.len() {
                dump(world, ty.parent, depth + 1, seen);
            }
            if ty.pointer != 0 && ty.pointer < world.types.len() {
                dump(world, ty.pointer, depth + 1, seen);
            }
        }
        for wanted in ["hknpBodyCinfo", "hknpShapeMassProperties", "hknpConvexPolytopeShape", "hknpConvexShape", "hknpShape", "hkpRagdollMotorConstraintAtom", "hkpConstraintMotor", "hkCompressedMassProperties", "hkpPositionConstraintMotor"] {
            if let Some(index) = world.types.iter().position(|t| t.name == wanted) {
                let mut fresh = std::collections::BTreeSet::new();
                dump(&world, index, 0, &mut fresh);
            }
        }
        for (index, item) in world.items.iter().enumerate() {
            let ty = &world.types[item.type_index];
            if ["hkVector4", "hkVector4f", "char", "hkUint8", "hkRefPtr"].contains(&ty.name.as_str()) {
                continue;
            }
            println!("ITEM {index} {} count={} offset={:#x}", ty.name, item.count, item.offset);
            dump(&world, item.type_index, 1, &mut seen);
            let at = world.data_start + item.offset;
            let size = ty.byte_size * item.count;
            let data = &bytes[at..at + size.min(0x400)];
            for row in (0..data.len()).step_by(16) {
                let end = (row + 16).min(data.len());
                let words: Vec<String> = data[row..end].chunks(4).map(|c| {
                    let mut w = [0u8; 4];
                    w[..c.len()].copy_from_slice(c);
                    format!("{:08x}", u32::from_le_bytes(w))
                }).collect();
                let floats: Vec<String> = data[row..end].chunks_exact(4).map(|c| format!("{:.4}", f32::from_le_bytes(c.try_into().unwrap()))).collect();
                println!("    +{row:04x} {}  {}", words.join(" "), floats.join(" "));
            }
        }
    }

    #[test]
    #[ignore = "audit; requires installed Dust District archives"]
    fn installed_cable_ragdoll_resource_layout() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        for path in catalog.paths_with_suffix(".ragdoll") {
            if !path.contains("cables_speaker") {
                continue;
            }
            let bytes = catalog.read(&path).unwrap().unwrap();
            println!("RAGDOLL {path} {} bytes head {:02x?}", bytes.len(), &bytes[..bytes.len().min(64)]);
            match parse_embedded_havok(&bytes) {
                Ok(world) => println!("  inventory {:?}", world.item_inventory()),
                Err(e) => println!("  not tagged: {e}"),
            }
        }
    }

}
