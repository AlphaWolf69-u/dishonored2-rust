//! Decode Void bimage7 mips and port the original texture streamer rules.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// FUN_1402eab10: image priority is a monotonic byte maximum, not a
/// replacement value. The owning material enumerator supplies the request.
pub fn raise_image_priority(priority: &mut u8, requested: u8) {
    if *priority < requested {
        *priority = requested;
    }
}

/// FUN_1402c2b90 clears the 21 format bits, retaining descriptor ownership
/// bits, and returns the same caller-owned word (not an allocated resource).
pub fn reset_resource_format(packed: &mut u32) -> &mut u32 {
    *packed &= 0xffe0_0000;
    packed
}

/// FUN_1402c2e30: the low six format-family bits determine upload/storage bpp.
pub fn image_bits_per_pixel(packed: u32) -> u32 {
    match packed & 63 {
        1 | 15 | 16 | 18 | 19 | 20 => 8,
        2 | 4 => 16,
        3 | 5 | 10 | 13 | 22 | 24 => 32,
        6 | 11 => 64,
        7 | 21 => 24,
        8 => 48,
        9 => 96,
        12 => 128,
        14 | 17 => 4,
        23 => 40,
        _ => 0,
    }
}

/// FUN_1402c2f50. Success can deliberately leave the caller's output unchanged;
/// retain that distinction rather than replacing it with a made-up GPU format.
pub fn map_image_format(family: u32, selector: i32, precision: u32, output: &mut u32) -> bool {
    let choose = |pairs: &[(i32, u32)]| pairs.iter().find(|(s, _)| *s == selector).map(|(_, v)| *v);
    let value = match family {
        0 => Some(0),
        1 => choose(&[
            (1, 60),
            (2, 64),
            (3, 62),
            (4, 63),
            (5, if precision == 1 { 61 } else { 65 }),
        ]),
        2 => choose(&[
            (1, 53),
            (2, 59),
            (3, 57),
            (4, 58),
            (5, 56),
            (7, if precision == 4 { 53 } else { 54 }),
        ]),
        3 => {
            if precision == 0 {
                return true;
            }
            if precision > 3 {
                if precision == 4 {
                    Some(if selector == 1 { 39 } else { 40 })
                } else if precision == 6 {
                    return false;
                } else {
                    return true;
                }
            } else {
                choose(&[(1, 39), (2, 43), (3, 42), (7, 41)])
            }
        }
        4 => choose(&[(1, 48), (2, 52), (3, 50), (4, 51), (5, 49)]),
        5 => choose(&[(1, 33), (2, 38), (3, 36), (4, 37), (5, 35), (7, 34)]),
        6 => choose(&[(1, 15), (2, 18), (3, 17), (7, 16)]),
        7 | 8 | 15 | 21 => None,
        9 => choose(&[(1, 5), (2, 8), (3, 7), (7, 6)]),
        10 => match precision {
            1 => choose(&[(1, 27), (2, 32), (3, 30), (4, 31), (5, 28), (6, 29)]),
            3 => choose(&[(1, 90), (5, 87), (6, 91)]),
            _ => None,
        },
        11 => choose(&[(1, 9), (2, 14), (3, 12), (4, 13), (5, 11), (7, 10)]),
        12 => choose(&[(1, 1), (2, 4), (3, 3), (7, 2)]),
        13 => choose(&[(1, 23), (3, 25), (5, 24)]),
        14 => choose(&[(1, 70), (5, 71), (6, 72)]),
        16 => choose(&[(1, 76), (5, 77), (6, 78)]),
        17 => choose(&[(1, 79), (4, 81), (5, 80)]),
        18 => choose(&[(1, 82), (4, 84), (5, 83)]),
        19 => choose(&[(1, 94), (2, 96), (3, 95)]),
        20 => choose(&[(1, 97), (5, 98), (6, 99)]),
        22 | 23 => {
            if precision == 5 {
                if selector != 3 {
                    return false;
                }
                Some(if family == 22 { 47 } else { 22 })
            } else if precision != 4 && precision != 6 {
                return true;
            } else if family == 22 {
                choose(&[(1, 44), (5, 46)])
            } else {
                choose(&[(1, 19), (7, 21)])
            }
        }
        24 => choose(&[(7, 26)]),
        _ => return true,
    };
    if let Some(value) = value {
        *output = value;
        true
    } else {
        false
    }
}

/// FUN_1402e98a0: auto-fill the four mip-count fields (+0x24..+0x30)
/// only when the first is zero. This is the original descriptor rule,
/// not the usual max-dimension full mip chain. Families14..20 stop at4x4.
pub fn initialize_image_mip_counts(family: u32, dimensions: [i32; 3], counts: &mut [i32; 4]) {
    if counts[0] != 0 {
        return;
    }
    let [mut width, mut height, mut depth] = dimensions;
    let has_depth = depth >= 2;
    let blocked = family.wrapping_sub(14) < 7;
    let mut count = 1;
    if width > 1 {
        while height > 1 && (!has_depth || depth > 1) {
            width >>= 1;
            height >>= 1;
            depth >>= 1;
            if blocked && (width < 4 || height < 4) {
                break;
            }
            count += 1;
            if width < 2 {
                break;
            }
        }
    }
    counts.fill(count);
}

/// FUN_1402c2ef0, on its defined writing/failure branches. Successful no-write
/// conversions otherwise read an uninitialized native stack local: reject them.
pub fn normalize_image_format(packed: &mut u32) -> Result<(), String> {
    let mut mapped = u32::MAX;
    if map_image_format(
        *packed & 63,
        ((*packed >> 6) & 15) as i32,
        (*packed >> 10) & 7,
        &mut mapped,
    ) {
        if mapped == u32::MAX {
            return Err("image format conversion has unspecified output".into());
        }
        *packed = (*packed & !0x1fe000) | ((mapped << 13) & 0x1fe000);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    // Uncompressed native intrinsics and BIM resources share the mip uploader.
    Rgba8,
    Rgba8Srgb,
    Rg8,
    R16Float,
    Rg32Float,
    Bgra8,
    Bc1,
    Bc1Srgb,
    Bc3,
    Bc3Srgb,
    Bc4,
    Bc4Snorm,
    Bc5,
    Bc5Snorm,
    Bc6Unsigned,
    Bc6Signed,
    Bc7,
    Bc7Srgb,
}

impl ImageFormat {
    pub fn from_dxgi(value: u32) -> Result<Self, String> {
        Ok(match value {
            28 => Self::Rgba8,
            29 => Self::Rgba8Srgb,
            49 => Self::Rg8,
            54 => Self::R16Float,
            16 => Self::Rg32Float,
            87 => Self::Bgra8,
            71 => Self::Bc1,
            72 => Self::Bc1Srgb,
            77 => Self::Bc3,
            78 => Self::Bc3Srgb,
            80 => Self::Bc4,
            81 => Self::Bc4Snorm,
            83 => Self::Bc5,
            84 => Self::Bc5Snorm,
            95 => Self::Bc6Unsigned,
            96 => Self::Bc6Signed,
            98 => Self::Bc7,
            99 => Self::Bc7Srgb,
            _ => return Err(format!("unsupported texture format {value}")),
        })
    }
    pub fn block_bytes(self) -> usize {
        match self {
            Self::Rgba8 | Self::Rgba8Srgb => 4,
            Self::Rg8 | Self::R16Float => 2,
            Self::Rg32Float => 8,
            Self::Bgra8 => 4,
            Self::Bc1 | Self::Bc1Srgb | Self::Bc4 | Self::Bc4Snorm => 8,
            _ => 16,
        }
    }
    pub fn bits_per_pixel(self) -> i32 {
        if self.block_width() == 1 {
            self.block_bytes() as i32 * 8
        } else {
            self.block_bytes() as i32 / 2
        }
    }
    pub fn block_width(self) -> u32 {
        match self {
            Self::Rgba8
            | Self::Rgba8Srgb
            | Self::Rg8
            | Self::R16Float
            | Self::Rg32Float
            | Self::Bgra8 => 1,
            _ => 4,
        }
    }
    pub fn mip_bytes(self, width: u32, height: u32) -> usize {
        width.div_ceil(self.block_width()) as usize
            * height.div_ceil(self.block_width()) as usize
            * self.block_bytes()
    }
}

/// The base BIM file owns the format. Separate mip headers contain no format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BimageInfo {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub packed_format: u32,
    pub format: ImageFormat,
}
impl BimageInfo {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..3) != Some(b"BIM") {
            return Err("not a BIM image".into());
        }
        let family = word(bytes, 12)?;
        let selector = word(bytes, 16)?;
        let precision = word(bytes, 20)?;
        if family > 63 || selector > 15 || precision > 7 {
            return Err("invalid BIM format fields".into());
        }
        let mut packed_format = family | selector << 6 | precision << 10;
        normalize_image_format(&mut packed_format)?;
        let format = ImageFormat::from_dxgi((packed_format >> 13) & 255)?;
        Ok(Self {
            width: word(bytes, 24)?,
            height: word(bytes, 28)?,
            mip_count: word(bytes, 44)?,
            packed_format,
            format,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedMip<'a> {
    pub width: u32,
    pub height: u32,
    pub payload: &'a [u8],
}

pub fn compressed_mip(
    bytes: &[u8],
    level: u32,
    format: ImageFormat,
) -> Result<CompressedMip<'_>, String> {
    let mip = BimageMipResource::parse(bytes)?;
    if mip.mip_level != level {
        return Err("bimage mip level differs from request".into());
    }
    let expected = format.mip_bytes(mip.width, mip.height);
    if mip.payload.len() != expected {
        return Err(format!(
            "invalid {format:?} mip payload: expected {expected}, got {}",
            mip.payload.len()
        ));
    }
    Ok(CompressedMip {
        width: mip.width,
        height: mip.height,
        payload: mip.payload,
    })
}

#[test]
fn rgba8_mips_keep_pixel_rows_and_color_space() {
    assert_eq!(ImageFormat::from_dxgi(28).unwrap(), ImageFormat::Rgba8);
    assert_eq!(ImageFormat::from_dxgi(29).unwrap(), ImageFormat::Rgba8Srgb);
    assert_eq!(ImageFormat::from_dxgi(49).unwrap().mip_bytes(3, 5), 30);
    assert_eq!(ImageFormat::from_dxgi(87).unwrap().mip_bytes(3, 5), 60);
    for f in [ImageFormat::Rgba8, ImageFormat::Rgba8Srgb] {
        assert_eq!(f.bits_per_pixel(), 32);
        let mut bytes = Vec::new();
        for v in [0u32, 0, 3, 5, 60] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&[127; 60]);
        let mip = compressed_mip(&bytes, 0, f).unwrap();
        assert_eq!((mip.width, mip.height, mip.payload.len()), (3, 5, 60));
    }
}

pub struct InlineCompressedChain {
    pub info: BimageInfo,
    pub payload: Vec<u8>,
}

/// A BIM cubemap stores one (level, face, width, height, byte count)
/// record for each of its six faces, with mip level as the outer order.
pub struct InlineCubeChain {
    pub info: BimageInfo,
    pub payload: Vec<u8>,
}
pub fn compressed_inline_cube(bytes: &[u8]) -> Result<InlineCubeChain, String> {
    let info = BimageInfo::parse(bytes)?;
    if word(bytes, 8)? != 2
        || word(bytes, 52)? != 6
        || info.width == 0
        || info.width != info.height
        || info.mip_count == 0
        || info.mip_count > 16
    {
        return Err("invalid BIM cubemap descriptor".into());
    }
    let mut at = 62usize;
    let mut payload = Vec::new();
    for level in 0..info.mip_count {
        let width = (info.width >> level).max(1);
        for face in 0..6 {
            let size = info.format.mip_bytes(width, width);
            let end = at
                .checked_add(20)
                .and_then(|n| n.checked_add(size))
                .ok_or("cube mip overflow")?;
            let record = bytes.get(at..end).ok_or("truncated cube mip")?;
            let mip = compressed_mip(record, level, info.format)?;
            if word(record, 4)? != face || mip.width != width || mip.height != width {
                return Err("cube face/mip descriptor mismatch".into());
            }
            payload.extend_from_slice(mip.payload);
            at = end;
        }
    }
    if at != bytes.len() {
        return Err("cube mip chain size mismatch".into());
    }
    Ok(InlineCubeChain { info, payload })
}

#[test]
#[ignore = "requires installed game archives"]
fn installed_reflection_cube_descriptors_and_faces_are_consistent() {
    let catalog =
        crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
    let mut count = 0;
    for path in catalog.paths_with_suffix(".probesnetwork") {
        let bytes = catalog.read_prefix(&path, 1024 * 1024).unwrap().unwrap();
        let (_, _, secondary, _) = crate::pnetwork::ordered_probe_names(&bytes).unwrap();
        let mut previous = None;
        for resource in secondary {
            let probe = crate::bprobe::parse(
                &catalog
                    .read(&format!("{resource}.bprobe"))
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let stem = crate::material::compiled_image_stem(&catalog, &probe.cubemap_name)
                .unwrap_or_else(|| panic!("{resource}: missing {}", probe.cubemap_name));
            let cube = compressed_inline_cube(&catalog.read(&stem).unwrap().unwrap())
                .unwrap_or_else(|e| panic!("{stem}: {e}"));
            assert_eq!(cube.info.format, ImageFormat::Bc6Unsigned, "{stem}");
            if let Some(info) = previous {
                assert_eq!(cube.info, info, "{stem}");
            }
            previous = Some(cube.info);
            assert!(
                probe
                    .position
                    .into_iter()
                    .chain(probe.virtual_planes.into_iter().flatten())
                    .all(f32::is_finite),
                "{resource}"
            );
            count += 1;
        }
    }
    assert!(count > 1000, "only {count} reflection cubes validated");
    println!("{count} installed reflection cubes validated across all networks");
}

#[test]
fn half_float_lookup_mips_preserve_authored_bits() {
    // FUN_1402c2f50: family 2, selector 7, precision 1 maps to DXGI 54.
    // Values outside [0,1] and subnormal values must survive without RGBA8 conversion.
    let words = [0x0000u16, 0x0001, 0x3555, 0x3c00, 0x4200, 0xbc00];
    let mut bytes = vec![0u8; 62];
    bytes[..3].copy_from_slice(b"BIM");
    for (offset, value) in [(12, 2u32), (16, 7), (20, 1), (24, 3), (28, 2), (44, 1)] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for value in [0u32, 0, 3, 2, 12] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in words {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let chain = compressed_inline_chain(&bytes).unwrap();
    assert_eq!(chain.info.format, ImageFormat::R16Float);
    assert_eq!(chain.info.format.bits_per_pixel(), 16);
    assert_eq!(chain.info.format.mip_bytes(3, 2), 12);
    assert_eq!(
        chain.payload,
        words
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    );
    bytes.pop();
    assert!(compressed_inline_chain(&bytes).is_err());
}
pub fn compressed_inline_chain(bytes: &[u8]) -> Result<InlineCompressedChain, String> {
    let info = BimageInfo::parse(bytes)?;
    if info.width == 0 || info.height == 0 || info.mip_count == 0 || info.mip_count > 16 {
        return Err("invalid BIM dimensions/mip count".into());
    }
    let mut at = 62usize;
    let mut payload = Vec::new();
    for level in 0..info.mip_count {
        let width = (info.width >> level).max(1);
        let height = (info.height >> level).max(1);
        let header = bytes
            .get(at..at + 20)
            .ok_or("truncated inline mip header")?;
        let size = word(header, 16)? as usize;
        let end = at
            .checked_add(20)
            .and_then(|at| at.checked_add(size))
            .ok_or("inline mip size overflow")?;
        let record = bytes.get(at..end).ok_or("truncated inline mip payload")?;
        let mip = compressed_mip(record, level, info.format)?;
        if (mip.width, mip.height) != (width, height) {
            return Err("inline mip dimensions disagree with BIM header".into());
        }
        payload.extend_from_slice(mip.payload);
        at = end;
    }
    if at != bytes.len() {
        return Err("inline mip chain size mismatch".into());
    }
    Ok(InlineCompressedChain { info, payload })
}

pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Header and compressed payload for one separate `.bimage7_mipN` resource.
/// Ghidra's FUN_14030e840 reads the five header words before handing the
/// resource to the upload worker; the mip index and final word are retained
/// so the caller can validate a streamed level against its request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BimageMipResource<'a> {
    pub mip_level: u32,
    pub header_flags: u32,
    pub width: u32,
    pub height: u32,
    pub payload: &'a [u8],
}

/// Validated BC7 block payload ready for direct GPU upload. The game streams
/// the authored block-compressed mip; expanding every 16-byte 4x4 block to 64
/// RGBA bytes before upload needlessly multiplies CPU and GPU image storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedBc7Mip<'a> {
    pub width: u32,
    pub height: u32,
    pub payload: &'a [u8],
}

fn validate_bc7_payload(width: u32, height: u32, payload: &[u8]) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("invalid BC7 mip dimensions".into());
    }
    let expected = width
        .div_ceil(4)
        .checked_mul(height.div_ceil(4))
        .and_then(|blocks| blocks.checked_mul(16))
        .ok_or("BC7 mip payload size overflow")? as usize;
    if payload.len() != expected {
        return Err(format!(
            "invalid BC7 mip payload: expected {expected} bytes for {width}x{height}, got {}",
            payload.len()
        ));
    }
    Ok(())
}

/// Select a separately stored BC7 mip without CPU decompression.
pub fn compressed_bc7_mip(
    bytes: &[u8],
    requested_level: u32,
) -> Result<CompressedBc7Mip<'_>, String> {
    let mip = BimageMipResource::parse(bytes)?;
    if mip.mip_level != requested_level {
        return Err(format!(
            "bimage mip header level {} does not match requested level {requested_level}",
            mip.mip_level
        ));
    }
    validate_bc7_payload(mip.width, mip.height, mip.payload)?;
    Ok(CompressedBc7Mip {
        width: mip.width,
        height: mip.height,
        payload: mip.payload,
    })
}

impl<'a> BimageMipResource<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        let mip_level = word(bytes, 0)?;
        let header_flags = word(bytes, 4)?;
        let width = word(bytes, 8)?;
        let height = word(bytes, 12)?;
        let payload_size = word(bytes, 16)? as usize;
        if width == 0 || height == 0 {
            return Err("invalid bimage mip dimensions".into());
        }
        let end = 20usize
            .checked_add(payload_size)
            .ok_or("bimage mip payload size overflow")?;
        let payload = bytes.get(20..end).ok_or("truncated bimage mip payload")?;
        if end != bytes.len() {
            return Err("bimage mip payload size mismatch".into());
        }
        Ok(Self {
            mip_level,
            header_flags,
            width,
            height,
            payload,
        })
    }
}

/// Area-only arithmetic model for FUN_14031a410. The game updates area and
/// packed metadata in one 64-bit compare/exchange; use
/// `PackedTextureVisibilityDemand` when those metadata bits are available.
/// This smaller type does not model the full streamer record.
#[derive(Debug, Default)]
pub struct ProjectedAreaDemand {
    area_bits: AtomicU32,
}

impl ProjectedAreaDemand {
    /// Submit one visible-material area metric, preserving the original
    /// compare/exchange retry behavior when workers submit concurrently.
    pub fn submit(&self, area: f32) {
        let mut observed = self.area_bits.load(Ordering::Relaxed);
        loop {
            let previous = f32::from_bits(observed);
            // The original COMISS/JBE sequence keeps the previous value for
            // both area <= previous and unordered comparisons (including
            // NaN). A negated `<=` test would incorrectly store NaN.
            let next = if area > previous { area } else { previous };
            match self.area_bits.compare_exchange_weak(
                observed,
                next.to_bits(),
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => observed = actual,
            }
        }
    }

    /// Read and clear this helper's area value. This explicit convenience
    /// operation does not model FUN_14030f3d0's conditional scheduling branch.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.area_bits.swap(0.0f32.to_bits(), Ordering::AcqRel))
    }

    pub fn area(&self) -> f32 {
        f32::from_bits(self.area_bits.load(Ordering::Acquire))
    }
}

/// Order requested images by the area component of FUN_14030f3d0's packed
/// score. With equal priority and frame metadata, the original reverse scan
/// handles the largest projected area first. Special image priorities and
/// the remaining packed metadata are not represented by this helper.
pub fn projected_area_request_order(
    requested: &[bool],
    demands: &[ProjectedAreaDemand],
) -> Result<Vec<usize>, String> {
    if requested.len() != demands.len() {
        return Err("texture request flags and area demands have different lengths".into());
    }
    let mut slots = requested
        .iter()
        .enumerate()
        .filter_map(|(slot, &needed)| needed.then_some(slot))
        .collect::<Vec<_>>();
    slots.sort_by(|&left, &right| {
        demands[right]
            .area()
            .total_cmp(&demands[left].area())
            .then_with(|| left.cmp(&right))
    });
    Ok(slots)
}

/// The 64-bit area/metadata word updated by FUN_14031a410 at streamer-record
/// offset +0x08. The low word is the maximum visible-area metric; the upper
/// word carries material mode bit 0, the image priority in bits 1..5, and
/// preserved fields in bits 6..14. This models the producer-side atomic
/// update only, not FUN_14030f3d0's scheduling and conditional consumption.
#[derive(Debug, Default)]
pub struct PackedTextureVisibilityDemand {
    word: AtomicU64,
}

impl PackedTextureVisibilityDemand {
    /// Match FUN_14031a410's 64-bit compare/exchange update. `material_flags`
    /// contributes only bit 0; `image_priority` contributes only bits 0..4.
    pub fn submit(&self, area: f32, material_flags: u8, image_priority: u8) {
        let mut observed = self.word.load(Ordering::Relaxed);
        loop {
            let old_area = f32::from_bits(observed as u32);
            // The executable's COMISS/JBE retains the old value for <= and
            // unordered inputs, including NaN.
            let next_area = if area > old_area { area } else { old_area };
            let old_metadata = (observed >> 32) as u32;
            let next_metadata = ((old_metadata & 0x7fc0)
                | (u32::from(image_priority & 0x1f) << 1)
                | u32::from(material_flags & 1))
                & 0x7ffe
                | u32::from(material_flags & 1);
            let next = (u64::from(next_metadata) << 32) | u64::from(next_area.to_bits());
            match self.word.compare_exchange_weak(
                observed,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => observed = actual,
            }
        }
    }

    pub fn area(&self) -> f32 {
        f32::from_bits(self.word.load(Ordering::Acquire) as u32)
    }

    pub fn metadata(&self) -> u16 {
        (self.word.load(Ordering::Acquire) >> 32) as u16
    }

    /// Model the storage operation that clears the low area word while
    /// retaining metadata. The original frame scheduler performs this only
    /// on its eligible scheduling branch; this method does not model that
    /// branch or decide when consumption occurs.
    pub fn clear_area(&self) -> f32 {
        let mut observed = self.word.load(Ordering::Relaxed);
        loop {
            let area = f32::from_bits(observed as u32);
            let next = observed & 0xffff_ffff_0000_0000;
            match self.word.compare_exchange_weak(
                observed,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return area,
                Err(actual) => observed = actual,
            }
        }
    }
}

/// FUN_14030f3d0 consumes the per-image maximum visible-area metric and
/// converts it to projected pixels before FUN_14030cce0 checks the next mip.
/// When DAT_1432efeb8 is zero, the image record's bits 1..5 supply the
/// priority multiplier; otherwise the multiplier is 1. The caller provides
/// that original global's state as `priority_multiplier_enabled`.
pub fn streamer_projected_pixels(
    area_metric: f32,
    priority_bits: u32,
    priority_multiplier_enabled: bool,
    render_width: i32,
    render_height: i32,
) -> f32 {
    let multiplier = if priority_multiplier_enabled {
        ((priority_bits >> 1) & 0x1f) as f32
    } else {
        1.0
    };
    area_metric * multiplier * render_width.wrapping_mul(render_height) as f32
}

/// The three image mip fields at +0x58, +0x5c, and +0x60 read by
/// FUN_14030d7a0 and FUN_14030d8d0. `queued` changes when an image update
/// is submitted; `resident` changes only when that update succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipResidency {
    pub resident: i32,
    pub queued: i32,
    pub minimum: i32,
}

impl MipResidency {
    /// Commit the resident level after the renderer reports a successful
    /// image update. FUN_14030d1e0 calls FUN_1402f5530 only after the upload
    /// work completes; FUN_1402f5530 copies the update descriptor's target
    /// level into the image's resident-mip field. Queueing alone must not
    /// advance residency.
    pub fn commit_successful_update(&mut self, target_mip: i32) -> Result<(), String> {
        if self.queued != target_mip {
            return Err("completed mip does not match the queued image update".into());
        }
        self.resident = target_mip;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipReductionRequest {
    pub target: i32,
    pub width: i32,
    pub height: i32,
    pub released_bytes: u32,
}

/// The 16-byte descriptor that FUN_14030f3d0 passes to FUN_14030c670.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MipUpdateDescriptor {
    pub image_token: u64,
    pub target_mip: i32,
    pub byte_estimate: u32,
}

/// Queue state encoded by idList's +0x0c control byte. Growth modes 1 and 2
/// do not grow from FUN_14030e700; mode 3 uses FUN_140577c80 and switches to
/// the mode-0 geometric policy (increment sentinel 0x3f) after a successful
/// reallocation.
#[derive(Debug)]
pub struct MipUpdateQueue {
    entries: Vec<MipUpdateDescriptor>,
    logical_capacity: u32,
    control: u8,
    linear_growth: bool,
}

impl MipUpdateQueue {
    pub fn new(logical_capacity: u32, control: u8, linear_growth: bool) -> Result<Self, String> {
        if logical_capacity > 0x00ff_ffff {
            return Err("texture update queue capacity exceeds its 24-bit field".into());
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(logical_capacity as usize)
            .map_err(|error| format!("cannot allocate texture update queue: {error}"))?;
        Ok(Self {
            entries,
            logical_capacity,
            control,
            linear_growth,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> u32 {
        self.logical_capacity
    }

    pub fn control(&self) -> u8 {
        self.control
    }

    pub fn entries(&self) -> &[MipUpdateDescriptor] {
        &self.entries
    }

    /// Append one descriptor, returning its prior count. None matches the
    /// original 0xffff_ffff result when the configured queue cannot grow.
    pub fn push(&mut self, descriptor: MipUpdateDescriptor) -> Option<u32> {
        if self.entries.len() as u32 == self.logical_capacity {
            let next_capacity = self.grown_capacity()?;
            if next_capacity > 0x00ff_ffff
                || self
                    .entries
                    .try_reserve_exact(next_capacity as usize - self.entries.len())
                    .is_err()
            {
                return None;
            }
            self.logical_capacity = next_capacity;
            if (self.control >> 6) == 3 {
                self.control = 0x3f;
            }
        }

        if self.entries.len() as u32 >= self.logical_capacity {
            return None;
        }
        let index = self.entries.len() as u32;
        self.entries.push(descriptor);
        Some(index)
    }

    fn grown_capacity(&self) -> Option<u32> {
        let current = self.logical_capacity;
        let growth_mode = self.control >> 6;
        if growth_mode != 0 && growth_mode != 3 {
            return None;
        }

        if self.linear_growth {
            return current.checked_add(1);
        }
        let increment = (self.control & 0x3f) as u32;
        if increment == 0x3f {
            return Some(if current == 0 {
                1
            } else {
                current.saturating_mul(2).min(i32::MAX as u32)
            });
        }
        let quantum = 1u32.checked_shl(increment)?;
        Some(current.checked_add(quantum)? & !(quantum - 1))
    }
}

/// Per-frame uploaded-byte threshold used by the original texture streamer.
/// FUN_14030d1e0 checks the configured KB value before starting each pending
/// mip, then adds that mip's uploaded bytes divided by 1000.0. Because the
/// check is before the next item, one upload may take the frame over budget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MipUploadBudget {
    budget_kb: f32,
    uploaded_kb: f32,
}

impl MipUploadBudget {
    pub fn new(budget_kb: f32) -> Self {
        Self {
            budget_kb,
            uploaded_kb: 0.0,
        }
    }

    /// Whether the game's loop would start another mip upload.
    pub fn allows_next_upload(&self) -> bool {
        self.uploaded_kb < self.budget_kb
    }

    /// Account for bytes uploaded by one completed mip request.
    pub fn record_upload(&mut self, bytes: i32) {
        self.uploaded_kb += bytes as f32 / 1000.0;
    }

    pub fn uploaded_kb(&self) -> f32 {
        self.uploaded_kb
    }
}

fn mip_bytes(bits_per_pixel: i32, width: i32, height: i32) -> u32 {
    (bits_per_pixel.wrapping_mul(width).wrapping_mul(height) as u32) >> 3
}

/// FUN_14030d7a0 prepares the request that drops all removable resident
/// mips. The original then submits this description to FUN_1402f4990.
pub fn queue_mip_drop_to_minimum(
    state: &mut MipResidency,
    bits_per_pixel: i32,
    width: i32,
    height: i32,
) -> Option<MipReductionRequest> {
    if state.queued != state.resident || state.minimum >= state.resident {
        return None;
    }
    let mut bytes = mip_bytes(bits_per_pixel, width, height);
    let mut released = bytes;
    for _ in state.minimum + 1..state.resident {
        bytes = ((bytes as i32) >> 2) as u32;
        released = released.wrapping_add(bytes);
    }
    state.queued = state.minimum;
    Some(MipReductionRequest {
        target: state.minimum,
        width,
        height,
        released_bytes: released,
    })
}

/// FUN_14030d8d0 prepares the request that drops one resident mip. The
/// submitted image dimensions are each halved with signed arithmetic shift.
pub fn queue_mip_drop_one(
    state: &mut MipResidency,
    bits_per_pixel: i32,
    width: i32,
    height: i32,
) -> Option<MipReductionRequest> {
    if state.queued != state.resident || state.minimum >= state.resident {
        return None;
    }
    state.queued -= 1;
    Some(MipReductionRequest {
        target: state.queued,
        width: width >> 1,
        height: height >> 1,
        released_bytes: mip_bytes(bits_per_pixel, width, height),
    })
}

/// The projected-area gate in the original FUN_14030cce0. This is one
/// condition for requesting a finer mip, not the complete streaming policy.
pub fn next_mip_meets_projected_area(projected_pixels: f32, width: u32, height: u32) -> bool {
    projected_pixels * 4.0 >= (width as f32) * (height as f32)
}

/// FUN_14030cce0's complete eligibility check for the next finer mip.
/// The width/height are the image's dimensions at `base_mip`; the original
/// image record stores those at +0xc/+0x8 and shifts them by the candidate
/// mip difference. `bits_per_pixel` comes from FUN_1402c2e30.
pub fn next_mip_is_eligible(
    current_mip: i32,
    maximum_mip: i32,
    base_mip: i32,
    base_width: i32,
    base_height: i32,
    bits_per_pixel: i32,
    projected_pixels: f32,
    bypass_area_gate: bool,
    max_texture_size_kb: i32,
) -> bool {
    let candidate = current_mip.wrapping_add(1);
    if candidate > maximum_mip {
        return false;
    }
    let shift = (candidate.wrapping_sub(base_mip) as u32) & 31;
    let width = base_width.wrapping_shl(shift);
    let height = base_height.wrapping_shl(shift);
    if !bypass_area_gate && projected_pixels * 4.0 < width.wrapping_mul(height) as f32 {
        return false;
    }
    let size_kb = (bits_per_pixel.wrapping_mul(width).wrapping_mul(height) as u32) >> 13;
    max_texture_size_kb == -1 || (size_kb as i32) <= max_texture_size_kb
}

/// Select and queue the next finer image mip as FUN_14030f3d0 does.
/// `eligibility_width/height` are the image record's +0x3c/+0x40 dimensions;
/// `upload_base_width/height` are its +0x4c/+0x50 dimensions used for the
/// queued byte estimate. `state.resident` is the base-mip index for the
/// projected-size test, while `state.queued + 1` is the candidate level.
///
/// The original aborts if the image already has an outstanding update
/// request. This function reports that invalid state as an error.
pub fn enqueue_next_mip_update(
    queue: &mut MipUpdateQueue,
    state: &mut MipResidency,
    image_token: u64,
    has_pending_update: bool,
    maximum_mip: i32,
    eligibility_width: i32,
    eligibility_height: i32,
    upload_base_width: i32,
    upload_base_height: i32,
    bits_per_pixel: i32,
    projected_pixels: f32,
    bypass_area_gate: bool,
    max_texture_size_kb: i32,
) -> Result<Option<u32>, String> {
    if has_pending_update {
        return Err("image already has a pending mip update".into());
    }

    let target_mip = state.queued.wrapping_add(1);
    if !next_mip_is_eligible(
        state.queued,
        maximum_mip,
        state.resident,
        eligibility_width,
        eligibility_height,
        bits_per_pixel,
        projected_pixels,
        bypass_area_gate,
        max_texture_size_kb,
    ) {
        return Ok(None);
    }

    let mut byte_estimate = mip_bytes(bits_per_pixel, upload_base_width, upload_base_height);
    let mut mip = target_mip;
    while mip < maximum_mip {
        byte_estimate = ((byte_estimate as i32) >> 2) as u32;
        mip = mip.wrapping_add(1);
    }
    let descriptor = MipUpdateDescriptor {
        image_token,
        target_mip,
        byte_estimate,
    };
    // FUN_14030f3d0 ignores FUN_14030c670's failure return and writes the
    // queued mip field afterward. Preserve that order even when the fixed
    // queue mode cannot grow and no descriptor is appended.
    let index = queue.push(descriptor);
    state.queued = target_mip;
    Ok(index)
}

fn word(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or("short bimage7 header")?
            .try_into()
            .unwrap(),
    ))
}

fn decode_blocks(bytes: &[u8], width: u32, height: u32, bc7: bool) -> Result<RgbaImage, String> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width % 4 != 0
        || height % 4 != 0
    {
        return Err(format!("unsupported texture dimensions {width}x{height}"));
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .ok_or("texture dimensions overflow")?;
    if bytes.len() != expected {
        return Err(format!(
            "unsupported texture payload: {} bytes for {width}x{height}",
            bytes.len()
        ));
    }
    let mut pixels = vec![0; expected * 4];
    let row_blocks = width as usize / 4;
    for by in 0..height as usize / 4 {
        for bx in 0..row_blocks {
            let start = (by * row_blocks + bx) * 16;
            let mut block = [0; 64];
            if bc7 {
                bcdec_rs::bc7(&bytes[start..start + 16], &mut block, 16);
            } else {
                bcdec_rs::bc3(&bytes[start..start + 16], &mut block, 16);
            }
            for row in 0..4 {
                let dest = ((by * 4 + row) * width as usize + bx * 4) * 4;
                pixels[dest..dest + 16].copy_from_slice(&block[row * 16..row * 16 + 16]);
            }
        }
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}

pub fn decode_bc7(bytes: &[u8]) -> Result<RgbaImage, String> {
    let mip = BimageMipResource::parse(bytes)?;
    decode_blocks(mip.payload, mip.width, mip.height, true)
}

/// Decode a separately stored mip only when its authored level matches the
/// request that selected the resource, as FUN_14030e840's filename does.
pub fn decode_bc7_mip(bytes: &[u8], requested_level: u32) -> Result<RgbaImage, String> {
    let mip = BimageMipResource::parse(bytes)?;
    if mip.mip_level != requested_level {
        return Err(format!(
            "bimage mip header level {} does not match requested level {requested_level}",
            mip.mip_level
        ));
    }
    decode_blocks(mip.payload, mip.width, mip.height, true)
}

fn inline_layout(bytes: &[u8], selected_mip: usize) -> Result<(u32, u32, usize, usize), String> {
    if bytes.get(0..3) != Some(b"BIM") {
        return Err("not a BIM image".into());
    }
    let width = word(bytes, 0x18)?;
    let height = word(bytes, 0x1c)?;
    let mip_count = word(bytes, 0x2c)? as usize;
    if mip_count == 0 || mip_count > 16 {
        return Err("unsupported inline mip count".into());
    }
    if selected_mip >= mip_count {
        return Err("inline mip index exceeds image".into());
    }
    let mut at = 62usize;
    let mut selected = (0u32, 0u32, 0usize, 0usize);
    for mip in 0..mip_count {
        let mip_width = (width >> mip).max(1) as usize;
        let mip_height = (height >> mip).max(1) as usize;
        let size = mip_width
            .div_ceil(4)
            .checked_mul(mip_height.div_ceil(4))
            .and_then(|blocks| blocks.checked_mul(16))
            .ok_or("inline mip size overflow")?;
        if word(bytes, at + 8)? as usize != mip_width
            || word(bytes, at + 12)? as usize != mip_height
            || word(bytes, at + 16)? as usize != size
        {
            return Err(format!("inline mip {mip} header disagrees with dimensions"));
        }
        at = at.checked_add(20).ok_or("inline header offset overflow")?;
        if mip == selected_mip {
            selected = (mip_width as u32, mip_height as u32, at, size);
        }
        at = at.checked_add(size).ok_or("inline image size overflow")?;
    }
    if bytes.len() != at {
        return Err(format!(
            "unsupported inline image size: expected {at}, got {}",
            bytes.len()
        ));
    }
    Ok(selected)
}

pub fn decode_inline_bc7(bytes: &[u8]) -> Result<RgbaImage, String> {
    decode_inline_bc7_mip(bytes, 0)
}

pub fn decode_inline_bc7_mip(bytes: &[u8], mip: usize) -> Result<RgbaImage, String> {
    let (width, height, start, size) = inline_layout(bytes, mip)?;
    decode_blocks(&bytes[start..start + size], width, height, true)
}

/// Select an inline BC7 mip while retaining its compressed authored blocks.
pub fn compressed_inline_bc7_mip(bytes: &[u8], mip: usize) -> Result<CompressedBc7Mip<'_>, String> {
    let (width, height, start, size) = inline_layout(bytes, mip)?;
    let payload = &bytes[start..start + size];
    validate_bc7_payload(width, height, payload)?;
    Ok(CompressedBc7Mip {
        width,
        height,
        payload,
    })
}

/// Copy the complete authored inline BC7 pyramid in GPU mip order, from its
/// largest inline level to its smallest. The separate `_mipN` resources can
/// be prepended by the caller as higher-resolution levels arrive.
pub struct InlineBc7MipChain {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub payload: Vec<u8>,
}

pub fn compressed_inline_bc7_chain(bytes: &[u8]) -> Result<InlineBc7MipChain, String> {
    let mip_count = word(bytes, 0x2c)?;
    if mip_count == 0 || mip_count > 16 {
        return Err("unsupported inline mip count".into());
    }
    let first = compressed_inline_bc7_mip(bytes, 0)?;
    let mut payload = Vec::new();
    for index in 0..mip_count as usize {
        let mip = compressed_inline_bc7_mip(bytes, index)?;
        payload.extend_from_slice(mip.payload);
    }
    Ok(InlineBc7MipChain {
        width: first.width,
        height: first.height,
        mip_count,
        payload,
    })
}

pub fn decode_inline_bc3(bytes: &[u8]) -> Result<RgbaImage, String> {
    decode_inline_bc3_mip(bytes, 0)
}

pub fn decode_inline_bc3_mip(bytes: &[u8], mip: usize) -> Result<RgbaImage, String> {
    let (width, height, start, size) = inline_layout(bytes, mip)?;
    decode_blocks(&bytes[start..start + size], width, height, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_descriptor_mip_counts_match() {
        let mut checked = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/image_mip_count_cases.txt"
        ))
        .lines()
        {
            let w: Vec<i32> = line
                .split_whitespace()
                .map(|v| v.parse().unwrap())
                .collect();
            let mut counts = [w[4], 71, 83, 97];
            initialize_image_mip_counts(w[0] as u32, [w[1], w[2], w[3]], &mut counts);
            assert_eq!(counts, [w[5], w[6], w[7], w[8]], "{line}");
            checked += 1;
        }
        assert_eq!(checked, 13650);
    }

    #[test]
    fn image_formats_match() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/image_format_cases.txt"
        ))
        .lines()
        {
            let f: Vec<_> = line.split_whitespace().collect();
            match f[0] {
                "F" => {
                    let mut output = 0xa55a7e19;
                    let ok = map_image_format(
                        f[1].parse().unwrap(),
                        f[2].parse().unwrap(),
                        f[3].parse().unwrap(),
                        &mut output,
                    );
                    assert_eq!(
                        (u8::from(ok), output),
                        (f[4].parse().unwrap(), f[5].parse().unwrap()),
                        "{line}"
                    );
                }
                "C" => {
                    let mut value = f[1].parse().unwrap();
                    let address = &mut value as *mut u32;
                    let returned = reset_resource_format(&mut value) as *mut u32;
                    assert_eq!(returned == address, f[3] == "1", "{line}");
                    assert_eq!(value, f[2].parse::<u32>().unwrap(), "{line}");
                }
                "B" => assert_eq!(
                    image_bits_per_pixel(f[1].parse().unwrap()),
                    f[2].parse::<u32>().unwrap(),
                    "{line}"
                ),
                "N" => {
                    let mut value = f[1].parse().unwrap();
                    normalize_image_format(&mut value).unwrap();
                    assert_eq!(value, f[2].parse::<u32>().unwrap(), "{line}");
                }
                _ => panic!("unexpected format fixture"),
            }
            count += 1;
        }
        assert!(count > 20000);
    }

    #[test]
    #[ignore = "requires the installed original game archives"]
    fn installed_bimage_formats_and_mips() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let root =
            "generated/image/models/environment/archi/generic/ext_stairs_set_01/ext_stairs_01";
        for (suffix, format) in [
            ("d", ImageFormat::Bc7Srgb),
            ("n", ImageFormat::Bc5Snorm),
            ("g", ImageFormat::Bc4),
        ] {
            let stem = format!("{root}_{suffix}.bimage7");
            let bytes = catalog.read(&stem).unwrap().unwrap();
            let chain = compressed_inline_chain(&bytes).unwrap();
            assert_eq!(chain.info.format, format);
            assert_eq!(
                (chain.info.width, chain.info.height, chain.info.mip_count),
                (512, 256, 7)
            );
            for level in 0..2 {
                let bytes = catalog
                    .read(&format!("{stem}_mip{level}"))
                    .unwrap()
                    .unwrap();
                let mip = compressed_mip(&bytes, level, format).unwrap();
                assert_eq!((mip.width, mip.height), (2048 >> level, 1024 >> level));
            }
        }
    }

    #[test]
    fn image_priority_matches_every_byte_pair() {
        let mut checked = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/material_binding_cases.txt"
        ))
        .lines()
        {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields[0] != "P" {
                continue;
            }
            let mut priority = fields[1].parse::<u8>().unwrap();
            raise_image_priority(&mut priority, fields[2].parse().unwrap());
            assert_eq!(priority, fields[3].parse::<u8>().unwrap(), "{line}");
            checked += 1;
        }
        assert_eq!(checked, 65536);
        let demand = PackedTextureVisibilityDemand::default();
        let mut priority = 0;
        for requested in [1, 16, 4] {
            raise_image_priority(&mut priority, requested);
        }
        demand.submit(0.25, 1, priority);
        assert_eq!((demand.metadata() >> 1) & 31, 16);
        assert_eq!(
            streamer_projected_pixels(demand.area(), u32::from(demand.metadata()), true, 100, 100),
            40000.0
        );
    }

    use std::sync::Arc;

    fn mip_update(image_token: u64) -> MipUpdateDescriptor {
        MipUpdateDescriptor {
            image_token,
            target_mip: 2,
            byte_estimate: 4096,
        }
    }

    #[test]
    fn mip_update_queue_appends_16_byte_descriptors() {
        assert_eq!(std::mem::size_of::<MipUpdateDescriptor>(), 16);
        let mut queue = MipUpdateQueue::new(2, 0, false).unwrap();
        assert_eq!(queue.push(mip_update(0x1122)), Some(0));
        assert_eq!(queue.push(mip_update(0x3344)), Some(1));
        assert_eq!(queue.entries(), &[mip_update(0x1122), mip_update(0x3344)]);
        assert_eq!(queue.capacity(), 2);
    }

    #[test]
    fn mip_update_queue_growth_matches_increment_and_rounding() {
        let mut rounded = MipUpdateQueue::new(5, 2, false).unwrap();
        for image in 0..5 {
            assert_eq!(rounded.push(mip_update(image)), Some(image as u32));
        }
        assert_eq!(rounded.push(mip_update(5)), Some(5));
        assert_eq!(rounded.capacity(), 8);

        let mut linear = MipUpdateQueue::new(2, 0, true).unwrap();
        assert_eq!(linear.push(mip_update(1)), Some(0));
        assert_eq!(linear.push(mip_update(2)), Some(1));
        assert_eq!(linear.push(mip_update(3)), Some(2));
        assert_eq!(linear.capacity(), 3);
    }

    #[test]
    fn mip_update_queue_special_growth_switches_to_doubling_mode() {
        let mut queue = MipUpdateQueue::new(2, 0xff, false).unwrap();
        assert_eq!(queue.push(mip_update(1)), Some(0));
        assert_eq!(queue.push(mip_update(2)), Some(1));
        assert_eq!(queue.push(mip_update(3)), Some(2));
        assert_eq!(queue.capacity(), 4);
        assert_eq!(queue.control(), 0x3f);
        for image in 4u64..7 {
            assert_eq!(queue.push(mip_update(image)), Some(image as u32 - 1));
        }
        assert_eq!(queue.push(mip_update(7)), Some(6));
        assert_eq!(queue.capacity(), 8);
    }

    #[test]
    fn mip_update_queue_fixed_modes_reject_growth() {
        for control in [0x40, 0x80] {
            let mut queue = MipUpdateQueue::new(1, control, false).unwrap();
            assert_eq!(queue.push(mip_update(1)), Some(0));
            assert_eq!(queue.push(mip_update(2)), None);
            assert_eq!(queue.len(), 1);
            assert_eq!(queue.capacity(), 1);
        }
    }

    #[test]
    fn mip_upload_budget_checks_before_each_upload_and_allows_one_item_overshoot() {
        let mut budget = MipUploadBudget::new(4096.0);
        assert!(budget.allows_next_upload());
        budget.record_upload(4_100_000);
        assert_eq!(budget.uploaded_kb(), 4100.0);
        assert!(!budget.allows_next_upload());

        assert!(!MipUploadBudget::new(0.0).allows_next_upload());
    }

    #[test]
    fn next_mip_request_uses_candidate_and_upload_byte_fields() {
        let mut queue = MipUpdateQueue::new(1, 0x40, false).unwrap();
        let mut state = MipResidency {
            resident: 1,
            queued: 1,
            minimum: 0,
        };

        // FUN_14030f3d0 tests the projected size using the +0x3c/+0x40
        // dimensions, but computes its queued descriptor bytes from +0x4c/+0x50.
        assert_eq!(
            enqueue_next_mip_update(
                &mut queue, &mut state, 0x1234, false, 3, 64, 64, 256, 256, 8, 16_384.0, false, 16,
            ),
            Ok(Some(0))
        );
        assert_eq!(
            queue.entries(),
            &[MipUpdateDescriptor {
                image_token: 0x1234,
                target_mip: 2,
                byte_estimate: 16_384,
            }]
        );
        assert_eq!(state.queued, 2);
    }

    #[test]
    fn mip_residency_advances_only_after_matching_upload_success() {
        let mut state = MipResidency {
            resident: 1,
            queued: 2,
            minimum: 0,
        };
        assert_eq!(state.resident, 1, "queueing does not make a mip resident");
        assert!(state.commit_successful_update(1).is_err());
        assert_eq!(state.resident, 1);
        state.commit_successful_update(2).unwrap();
        assert_eq!(state.resident, 2);
        assert_eq!(state.queued, 2);
    }

    #[test]
    fn next_mip_request_preserves_state_when_rejected_or_already_pending() {
        let mut queue = MipUpdateQueue::new(1, 0x40, false).unwrap();
        let mut state = MipResidency {
            resident: 1,
            queued: 1,
            minimum: 0,
        };
        assert_eq!(
            enqueue_next_mip_update(
                &mut queue, &mut state, 1, false, 3, 64, 64, 256, 256, 8, 1.0, false, 16,
            ),
            Ok(None)
        );
        assert_eq!(queue.len(), 0);
        assert_eq!(state.queued, 1);

        assert!(enqueue_next_mip_update(
            &mut queue, &mut state, 1, true, 3, 64, 64, 256, 256, 8, 16_384.0, false, 16,
        )
        .is_err());
        assert_eq!(queue.len(), 0);
        assert_eq!(state.queued, 1);
    }

    #[test]
    fn next_mip_request_advances_image_even_when_queue_cannot_grow() {
        let mut queue = MipUpdateQueue::new(1, 0x40, false).unwrap();
        assert_eq!(queue.push(mip_update(0)), Some(0));
        let mut state = MipResidency {
            resident: 1,
            queued: 1,
            minimum: 0,
        };
        assert_eq!(
            enqueue_next_mip_update(
                &mut queue, &mut state, 1, false, 3, 64, 64, 256, 256, 8, 16_384.0, false, 16,
            ),
            Ok(None)
        );
        assert_eq!(state.queued, 2);
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn projected_area_demand_keeps_concurrent_max_and_resets_on_take() {
        let demand = Arc::new(ProjectedAreaDemand::default());
        let workers = (1..=16)
            .map(|area| {
                let demand = Arc::clone(&demand);
                std::thread::spawn(move || demand.submit(area as f32 / 4.0))
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(demand.take(), 4.0);
        assert_eq!(demand.take(), 0.0);
    }

    #[test]
    fn projected_area_demand_ignores_unordered_nan_submission() {
        let demand = ProjectedAreaDemand::default();
        demand.submit(0.25);
        demand.submit(f32::NAN);
        assert_eq!(demand.take(), 0.25);
    }

    #[test]
    fn diffuse_requests_follow_descending_projected_area() {
        let demands = (0..4)
            .map(|_| ProjectedAreaDemand::default())
            .collect::<Vec<_>>();
        demands[0].submit(0.25);
        demands[1].submit(0.75);
        demands[2].submit(0.75);
        demands[3].submit(0.5);

        assert_eq!(
            projected_area_request_order(&[true, true, false, true], &demands),
            Ok(vec![1, 3, 0])
        );
        assert!(projected_area_request_order(&[true], &demands).is_err());
    }

    #[test]
    fn packed_visibility_demand_updates_area_and_metadata_with_one_cas() {
        let demand = PackedTextureVisibilityDemand::default();
        demand.word.store(
            (u64::from(0x0150u16) << 32) | u64::from(1.0f32.to_bits()),
            Ordering::Relaxed,
        );

        demand.submit(2.0, 1, 3);
        assert_eq!(demand.area(), 2.0);
        assert_eq!(demand.metadata(), 0x0147);

        // Lower area retains the maximum; requested priority and mode replace
        // their fields while bits 6..14 remain from the old metadata word.
        demand.submit(0.5, 0, 2);
        assert_eq!(demand.area(), 2.0);
        assert_eq!(demand.metadata(), 0x0144);

        assert_eq!(demand.clear_area(), 2.0);
        assert_eq!(demand.area(), 0.0);
        assert_eq!(demand.metadata(), 0x0144);
    }

    #[test]
    fn packed_visibility_demand_keeps_area_on_unordered_submission() {
        let demand = PackedTextureVisibilityDemand::default();
        demand.submit(0.25, 0, 1);
        demand.submit(f32::NAN, 1, 4);
        assert_eq!(demand.area(), 0.25);
        assert_eq!(demand.metadata(), 9);
    }

    #[test]
    fn streamer_scales_visible_area_by_priority_and_render_pixels() {
        assert_eq!(
            streamer_projected_pixels(0.25, 6, false, 1920, 1080),
            518_400.0
        );
        assert_eq!(
            streamer_projected_pixels(0.25, 6, true, 1920, 1080),
            1_555_200.0
        );
    }

    #[test]
    fn mip_drop_requests_wait_for_previous_update() {
        let mut one = MipResidency {
            resident: 3,
            queued: 3,
            minimum: 1,
        };
        assert_eq!(
            queue_mip_drop_one(&mut one, 8, 64, 64),
            Some(MipReductionRequest {
                target: 2,
                width: 32,
                height: 32,
                released_bytes: 4096,
            })
        );
        assert_eq!(
            one,
            MipResidency {
                resident: 3,
                queued: 2,
                minimum: 1
            }
        );
        assert_eq!(queue_mip_drop_one(&mut one, 8, 64, 64), None);
        let mut all = MipResidency {
            resident: 3,
            queued: 3,
            minimum: 1,
        };
        assert_eq!(
            queue_mip_drop_to_minimum(&mut all, 8, 64, 64),
            Some(MipReductionRequest {
                target: 1,
                width: 64,
                height: 64,
                released_bytes: 5120,
            })
        );
        assert_eq!(queue_mip_drop_to_minimum(&mut all, 8, 64, 64), None);
    }

    #[test]
    fn projected_area_gate_matches_threshold() {
        assert!(!next_mip_meets_projected_area(1023.0, 64, 64));
        assert!(next_mip_meets_projected_area(1024.0, 64, 64));
    }

    #[test]
    fn next_mip_gate_checks_count_area_and_size() {
        let gate = |current, pixels, bypass, max_kb| {
            next_mip_is_eligible(current, 1, 0, 64, 64, 8, pixels, bypass, max_kb)
        };
        assert!(!gate(1, 4096.0, false, 16));
        assert!(!gate(0, 4095.0, false, 16));
        assert!(gate(0, 4096.0, false, 16));
        assert!(!gate(0, 4096.0, false, 15));
        assert!(gate(0, 0.0, true, 16));
        assert!(gate(0, 4096.0, false, -1));
    }
    #[test]
    fn rejects_short_mip() {
        assert!(decode_bc7(&[0; 4]).is_err());
    }

    #[test]
    fn parses_separate_mip_level_and_compressed_payload_extent() {
        let mut bytes = Vec::new();
        for value in [2u32, 0, 4, 4, 16] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[0; 16]);
        let mip = BimageMipResource::parse(&bytes).unwrap();
        assert_eq!(mip.mip_level, 2);
        assert_eq!(mip.header_flags, 0);
        assert_eq!((mip.width, mip.height), (4, 4));
        assert_eq!(mip.payload.len(), 16);
        let compressed = compressed_bc7_mip(&bytes, 2).unwrap();
        assert_eq!((compressed.width, compressed.height), (4, 4));
        assert_eq!(compressed.payload, &[0; 16]);
        assert_eq!(decode_bc7_mip(&bytes, 2).unwrap().pixels.len(), 64);
        assert!(decode_bc7_mip(&bytes, 1).is_err());
        assert!(compressed_bc7_mip(&bytes, 1).is_err());

        let mut invalid_extent = bytes.clone();
        invalid_extent[8..12].copy_from_slice(&8u32.to_le_bytes());
        assert!(BimageMipResource::parse(&invalid_extent).is_ok());
        assert!(compressed_bc7_mip(&invalid_extent, 2).is_err());

        bytes.pop();
        assert!(BimageMipResource::parse(&bytes).is_err());
    }

    #[test]
    fn rejects_short_inline_image() {
        assert!(decode_inline_bc7(b"BIM").is_err());
    }
    #[test]
    fn inline_mip_headers_are_interleaved_with_payloads() {
        let mut bytes = vec![0; 62];
        bytes[0..3].copy_from_slice(b"BIM");
        bytes[0x18..0x1c].copy_from_slice(&8u32.to_le_bytes());
        bytes[0x1c..0x20].copy_from_slice(&8u32.to_le_bytes());
        bytes[0x2c..0x30].copy_from_slice(&2u32.to_le_bytes());
        for (width, height, size) in [(8u32, 8u32, 64usize), (4, 4, 16)] {
            let mut header = [0u8; 20];
            header[8..12].copy_from_slice(&width.to_le_bytes());
            header[12..16].copy_from_slice(&height.to_le_bytes());
            header[16..20].copy_from_slice(&(size as u32).to_le_bytes());
            bytes.extend_from_slice(&header);
            bytes.extend(std::iter::repeat_n(0u8, size));
        }
        assert_eq!(inline_layout(&bytes, 0).unwrap(), (8, 8, 82, 64));
        assert_eq!(inline_layout(&bytes, 1).unwrap(), (4, 4, 166, 16));
        let compressed = compressed_inline_bc7_mip(&bytes, 1).unwrap();
        assert_eq!((compressed.width, compressed.height), (4, 4));
        assert_eq!(compressed.payload, &[0; 16]);
        assert!(inline_layout(&bytes, 2).is_err());
        bytes.pop();
        assert!(inline_layout(&bytes, 1).is_err());
    }
}
