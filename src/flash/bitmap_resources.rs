//! Library bitmap residency. GDraw `1402e2c00` uploads raw iggytex blocks
//! directly (format 13 -> DXGI_FORMAT_BC7_UNORM), without an RGBA copy.
use crate::assets::{archive::Catalog, iggy::IggyFile, texture::ImageFormat};
use std::{ops::{Deref, Range}, sync::Arc};

/// Installed contiguous blocks can share their source member; decoded or
/// partial records own their existing per-image output.
pub enum ImageBytes {
    Shared { source: Arc<Vec<u8>>, range: Range<usize> },
    Owned(Box<[u8]>),
}
impl Deref for ImageBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Shared { source, range } => &source[range.clone()],
            Self::Owned(bytes) => bytes,
        }
    }
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    /// Authored GPU format. The GDraw canvas samples display colours as UNORM.
    pub format: ImageFormat,
    pub bytes: ImageBytes,
}

pub fn load(catalog: &Catalog, resource: &str, file: &IggyFile) -> Result<Vec<(u16, Image)>, String> {
    let entries = file.textures().map_err(|e| e.to_string())?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let started = std::time::Instant::now();
    let path = resource.replace(".iggy", ".raw.iggytex");
    let mut data = catalog.read(&path).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} not indexed"))?;
    let source_bytes = data.len();
    let source_capacity = data.capacity();
    let expanded = std::env::var("DH2_DEBUG_UI_EXPANDED_BITMAPS").as_deref() == Ok("1");
    let single_mip = std::env::var("DH2_DEBUG_UI_SINGLE_MIP").as_deref() == Ok("1");
    let copy = std::env::var("DH2_DEBUG_COPY_UI_BITMAPS").as_deref() == Ok("1");
    let signatures = std::env::var("DH2_DEBUG_UI_BITMAP_SIGNATURES").as_deref() == Ok("1");
    // Share only a complete partition: retaining padding, unused records or
    // decoded fallback sources would otherwise increase resident payload.
    let mut ranges = Vec::with_capacity(entries.len());
    let mut eligible = !copy;
    for entry in &entries {
        let mut candidate = *entry;
        if single_mip { candidate.mips = 1; }
        if candidate.width == 0 || candidate.height == 0
            || u32::from(candidate.mips.max(1)) > u32::from(candidate.width.max(candidate.height)).ilog2() + 1 {
            eligible = false; break;
        }
        let direct = candidate.format == 0 || (candidate.format == 13
            && candidate.width % 4 == 0 && candidate.height % 4 == 0 && !expanded);
        let range = candidate.byte_len().and_then(|len| (candidate.offset as usize).checked_add(len)
            .map(|end| candidate.offset as usize..end));
        if !direct || range.as_ref().is_none_or(|range| range.end > source_bytes) {
            eligible = false; break;
        }
        ranges.push(range.unwrap());
    }
    ranges.sort_unstable_by_key(|range| range.start);
    let mut end = 0;
    for range in &ranges {
        if range.start != end { eligible = false; break; }
        end = range.end;
    }
    eligible &= end == source_bytes;
    let shared = eligible.then(|| Arc::new(std::mem::take(&mut data)));
    let input = shared.as_deref().unwrap_or(&data);
    let mut copied_bytes = 0usize;
    let mut copied_images = 0usize;
    let mut images = Vec::with_capacity(entries.len());
    let mut rgba_bytes = 0usize;
    for mut entry in entries {
        let (width, height) = (entry.width as u32, entry.height as u32);
        if width == 0 || height == 0 { return Err("empty iggytex image".into()); }
        if single_mip { entry.mips = 1; }
        let mip_count = u32::from(entry.mips.max(1));
        if mip_count > width.max(height).ilog2() + 1 { return Err("iggytex mip count exceeds its dimensions".into()); }
        rgba_bytes += (0..mip_count).map(|mip| (width >> mip).max(1) as usize * (height >> mip).max(1) as usize * 4).sum::<usize>();
        // Host: wgpu requires block-aligned base extents. Keep the existing
        // cropped decode for nonaligned bitmaps; never pad/stretch their UVs.
        let (format, bytes) = if entry.format == 13 && width % 4 == 0 && height % 4 == 0 && !expanded {
            let size = entry.byte_len().ok_or("unsupported iggytex block format")?;
            let start = entry.offset as usize;
            let end = start.checked_add(size).ok_or("iggytex entry overflow")?;
            let bytes = input.get(start..end).ok_or("iggytex entry outside file")?;
            let pixels = if let Some(source) = &shared {
                ImageBytes::Shared { source: source.clone(), range: start..end }
            } else {
                copied_bytes += bytes.len(); copied_images += 1;
                ImageBytes::Owned(bytes.to_vec().into_boxed_slice())
            };
            (ImageFormat::Bc7, pixels)
        } else if entry.format == 0 {
            let size = entry.byte_len().ok_or("unsupported iggytex block format")?;
            let start = entry.offset as usize;
            let end = start.checked_add(size).ok_or("iggytex entry overflow")?;
            let bytes = input.get(start..end).ok_or("iggytex entry outside file")?;
            let pixels = if let Some(source) = &shared {
                ImageBytes::Shared { source: source.clone(), range: start..end }
            } else {
                copied_bytes += bytes.len(); copied_images += 1;
                ImageBytes::Owned(bytes.to_vec().into_boxed_slice())
            };
            (ImageFormat::Rgba8, pixels)
        } else {
            let mut pixels = Vec::new();
            let mut offset = entry.offset;
            for mip in 0..mip_count {
                let level = crate::assets::iggy::shape::TextureEntry {
                    offset, width: (width >> mip).max(1) as u16, height: (height >> mip).max(1) as u16, mips: 1, ..entry
                };
                pixels.extend(super::raster::decode_entry(input, &level)?);
                offset = offset.checked_add(u32::try_from(level.byte_len().ok_or("unsupported iggytex block format")?).map_err(|_| "iggytex mip size overflow")?)
                    .ok_or("iggytex mip offset overflow")?;
            }
            copied_bytes += pixels.len(); copied_images += 1;
            (ImageFormat::Rgba8, ImageBytes::Owned(pixels.into_boxed_slice()))
        };
        if signatures {
            let mut crc = flate2::Crc::new(); crc.update(&bytes);
            eprintln!("MEMORY UI bitmap signature: resource={resource} character={} extent={width}x{height} mips={mip_count} format={format:?} bytes={} crc={:08x}", entry.character, bytes.len(), crc.sum());
        }
        images.push((entry.character, Image { width, height, mip_count, format, bytes }));
    }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY UI bitmap backing: resource={resource} shared={} source_bytes={source_bytes} copied_bytes={copied_bytes} copied_images={copied_images} retained_source_bytes={} retained_source_capacity={}", shared.is_some(), if shared.is_some() { source_bytes } else { 0 }, if shared.is_some() { source_capacity } else { 0 });
        println!("MEMORY UI bitmaps: resource={resource} bytes={} rgba_bytes={rgba_bytes} images={} mip_levels={} load_ms={:.3}",
            images.iter().map(|(_, image)| image.bytes.len()).sum::<usize>(), images.len(), images.iter().map(|(_, image)| image.mip_count).sum::<u32>(), started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(images)
}

/// Process resident library images. The resource registry returns the existing
/// payload on reacquisition (1400607f0 -> 14005feb0); live movie state remains
/// separate. Share immutable compressed images across loading and pause players.
/// bevy_backend: Arc owns the payload independently of either movie thread.
type LibraryImages = std::sync::Arc<Vec<(u16, std::sync::Arc<Image>)>>;
static RESIDENT: std::sync::Mutex<Option<std::collections::HashMap<(u64, String), LibraryImages>>> = std::sync::Mutex::new(None);

pub fn acquire(catalog: &Catalog, resource: &str, file: &IggyFile) -> Result<LibraryImages, String> {
    let key = (catalog.identity(), resource.replace('\\', "/").to_ascii_lowercase());
    let mut resident = RESIDENT.lock().unwrap();
    let resident = resident.get_or_insert_with(Default::default);
    // Movie/image leases keep their data if another catalog becomes active.
    resident.retain(|(identity, _), _| *identity == catalog.identity());
    // Both the library's texture table and its companion image member can
    // come from an editable layer while this catalog identity remains stable.
    let mutable = catalog.has_overlays() && (catalog.has_overlay_file(resource)
        || catalog.has_overlay_file(&resource.replace(".iggy", ".raw.iggytex")));
    if !mutable {
        if let Some(images) = resident.get(&key) { return Ok(images.clone()); }
    }
    let images: LibraryImages = std::sync::Arc::new(load(catalog, resource, file)?.into_iter()
        .map(|(id, image)| (id, std::sync::Arc::new(image))).collect());
    if !mutable { resident.insert(key, images.clone()); }
    Ok(images)
}
