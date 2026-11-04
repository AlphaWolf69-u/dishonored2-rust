//! Library bitmap residency. GDraw `1402e2c00` uploads raw iggytex blocks
//! directly (format 13 -> DXGI_FORMAT_BC7_UNORM), without an RGBA copy.
use crate::assets::{archive::Catalog, iggy::IggyFile, texture::ImageFormat};

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    /// Authored GPU format. The GDraw canvas samples display colours as UNORM.
    pub format: ImageFormat,
    pub bytes: Box<[u8]>,
}

pub fn load(catalog: &Catalog, resource: &str, file: &IggyFile) -> Result<Vec<(u16, Image)>, String> {
    let entries = file.textures().map_err(|e| e.to_string())?;
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let started = std::time::Instant::now();
    let path = resource.replace(".iggy", ".raw.iggytex");
    let data = catalog.read(&path).map_err(|e| e.to_string())?.ok_or_else(|| format!("{path} not indexed"))?;
    let expanded = std::env::var("DH2_DEBUG_UI_EXPANDED_BITMAPS").as_deref() == Ok("1");
    let mut images = Vec::with_capacity(entries.len());
    let mut rgba_bytes = 0usize;
    for mut entry in entries {
        let (width, height) = (entry.width as u32, entry.height as u32);
        if width == 0 || height == 0 { return Err("empty iggytex image".into()); }
        if std::env::var("DH2_DEBUG_UI_SINGLE_MIP").as_deref() == Ok("1") { entry.mips = 1; }
        let mip_count = u32::from(entry.mips.max(1));
        if mip_count > width.max(height).ilog2() + 1 { return Err("iggytex mip count exceeds its dimensions".into()); }
        rgba_bytes += (0..mip_count).map(|mip| (width >> mip).max(1) as usize * (height >> mip).max(1) as usize * 4).sum::<usize>();
        // Host: wgpu requires block-aligned base extents. Keep the existing
        // cropped decode for nonaligned bitmaps; never pad/stretch their UVs.
        let (format, bytes) = if entry.format == 13 && width % 4 == 0 && height % 4 == 0 && !expanded {
            let size = entry.byte_len().ok_or("unsupported iggytex block format")?;
            let start = entry.offset as usize;
            (ImageFormat::Bc7, data.get(start..start + size).ok_or("iggytex entry outside file")?.to_vec())
        } else if entry.format == 0 {
            let size = entry.byte_len().ok_or("unsupported iggytex block format")?;
            let start = entry.offset as usize;
            (ImageFormat::Rgba8, data.get(start..start + size).ok_or("iggytex entry outside file")?.to_vec())
        } else {
            let mut pixels = Vec::new();
            let mut offset = entry.offset;
            for mip in 0..mip_count {
                let level = crate::assets::iggy::shape::TextureEntry {
                    offset, width: (width >> mip).max(1) as u16, height: (height >> mip).max(1) as u16, mips: 1, ..entry
                };
                pixels.extend(super::raster::decode_entry(&data, &level)?);
                offset = offset.checked_add(u32::try_from(level.byte_len().ok_or("unsupported iggytex block format")?).map_err(|_| "iggytex mip size overflow")?)
                    .ok_or("iggytex mip offset overflow")?;
            }
            (ImageFormat::Rgba8, pixels)
        };
        images.push((entry.character, Image { width, height, mip_count, format, bytes: bytes.into_boxed_slice() }));
    }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
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
static RESIDENT: std::sync::Mutex<Option<std::collections::HashMap<(std::path::PathBuf, String), LibraryImages>>> = std::sync::Mutex::new(None);

pub fn acquire(catalog: &Catalog, resource: &str, file: &IggyFile) -> Result<LibraryImages, String> {
    let key = (catalog.base().to_owned(), resource.replace('\\', "/").to_ascii_lowercase());
    let mut resident = RESIDENT.lock().unwrap();
    let resident = resident.get_or_insert_with(Default::default);
    if let Some(images) = resident.get(&key) { return Ok(images.clone()); }
    let images: LibraryImages = std::sync::Arc::new(load(catalog, resource, file)?.into_iter()
        .map(|(id, image)| (id, std::sync::Arc::new(image))).collect());
    resident.insert(key, images.clone());
    Ok(images)
}
