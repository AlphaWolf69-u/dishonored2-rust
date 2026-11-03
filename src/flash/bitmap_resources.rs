//! Library bitmap residency. GDraw `1402e2c00` uploads raw iggytex blocks
//! directly (format 13 -> DXGI_FORMAT_BC7_UNORM), without an RGBA copy.
use crate::assets::{archive::Catalog, iggy::IggyFile};

pub struct Image {
    pub width: u32,
    pub height: u32,
    /// GDraw raw format: 0 RGBA8, 13 BC7. UI composition stays UNORM.
    pub format: u8,
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
    for entry in entries {
        let (width, height) = (entry.width as u32, entry.height as u32);
        rgba_bytes += width as usize * height as usize * 4;
        // Host: wgpu requires block-aligned base extents. Keep the existing
        // cropped decode for nonaligned bitmaps; never pad/stretch their UVs.
        let (format, bytes) = if entry.format == 13 && width % 4 == 0 && height % 4 == 0 && !expanded {
            let size = width as usize / 4 * (height as usize / 4) * 16;
            let start = entry.offset as usize;
            (13, data.get(start..start + size).ok_or("iggytex entry outside file")?.to_vec())
        } else {
            (0, super::raster::decode_entry(&data, &entry)?)
        };
        images.push((entry.character, Image { width, height, format, bytes: bytes.into_boxed_slice() }));
    }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        println!("MEMORY UI bitmaps: resource={resource} bytes={} rgba_bytes={rgba_bytes} images={} load_ms={:.3}",
            images.iter().map(|(_, image)| image.bytes.len()).sum::<usize>(), images.len(), started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(images)
}
