//! Library bitmap residency. GDraw `1402e2c00` uploads raw iggytex blocks
//! directly (format 13 -> DXGI_FORMAT_BC7_UNORM), without an RGBA copy.
use crate::assets::{archive::Catalog, iggy::IggyFile, texture::ImageFormat};
use std::{collections::HashMap, ops::{Deref, Range}, sync::{Arc, Weak}};

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

type BitmapEntries = Vec<(u16, Arc<Image>)>;
pub(crate) type LibraryImages = Arc<BitmapEntries>;

/// A movie map and its submitted frames lease the exact immutable packs
/// whose bitmap entries they publish.
#[derive(Clone, Default)]
pub struct MovieBitmaps {
    characters: HashMap<super::player::CharRef, Arc<Image>>,
    packs: Vec<LibraryImages>,
}

impl MovieBitmaps {
    pub(crate) fn insert_pack(&mut self, library: usize, pack: LibraryImages) {
        self.characters.extend(pack.iter().map(|(id, image)| {
            (super::player::CharRef { library, id: *id }, image.clone())
        }));
        self.packs.push(pack);
    }

    pub(crate) fn append_layer(&mut self, layer: Self, library_offset: usize) {
        self.characters.extend(layer.characters.into_iter().map(|(mut character, image)| {
            character.library += library_offset;
            (character, image)
        }));
        self.packs.extend(layer.packs);
    }
}

impl Deref for MovieBitmaps {
    type Target = HashMap<super::player::CharRef, Arc<Image>>;

    fn deref(&self) -> &Self::Target { &self.characters }
}

enum CachedLibrary {
    Live(Weak<BitmapEntries>),
    /// Same-executable observation of the previous session retention.
    Session(LibraryImages),
}

static RESIDENT: std::sync::Mutex<Option<HashMap<(u64, String), CachedLibrary>>> =
    std::sync::Mutex::new(None);

fn retain_session_payload() -> bool {
    static RETAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RETAIN.get_or_init(|| std::env::var_os("DH2_DEBUG_RETAIN_UI_BITMAPS").is_some_and(|value| value == "1"))
}

/// Retire cache references only; submitted movies keep their own image Arcs.
pub(crate) fn retire_catalog(catalog: u64) -> usize {
    let mut resident = RESIDENT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(images) = resident.as_mut() else { return 0; };
    let before = images.len();
    images.retain(|(identity, _), _| *identity != catalog);
    let removed = before - images.len();
    if images.is_empty() { *resident = None; }
    removed
}

/// Diagnostic lookup for strong session ownership.
pub(crate) fn debug_resident(catalog: u64, resource: &str) -> bool {
    let key = (catalog, resource.replace('\\', "/").to_ascii_lowercase());
    matches!(
        RESIDENT.lock().unwrap_or_else(|error| error.into_inner())
            .as_ref().and_then(|resident| resident.get(&key)),
        Some(CachedLibrary::Session(_))
    )
}

pub fn acquire(catalog: &Catalog, resource: &str, file: &IggyFile) -> Result<LibraryImages, String> {
    // Both the texture table and its companion member may come from an
    // editable layer. Acquire that generation independently.
    let mutable = catalog.has_overlays() && (catalog.has_overlay_file(resource)
        || catalog.has_overlay_file(&resource.replace(".iggy", ".raw.iggytex")));
    if mutable {
        return Ok(Arc::new(load(catalog, resource, file)?.into_iter()
            .map(|(id, image)| (id, Arc::new(image))).collect()));
    }
    let key = (catalog.identity(), resource.replace('\\', "/").to_ascii_lowercase());
    let mut resident = RESIDENT.lock().unwrap();
    let resident = resident.get_or_insert_with(Default::default);
    resident.retain(|_, entry| match entry {
        CachedLibrary::Live(images) => images.strong_count() != 0,
        CachedLibrary::Session(_) => true,
    });
    match resident.get(&key) {
        Some(CachedLibrary::Session(images)) => return Ok(images.clone()),
        Some(CachedLibrary::Live(images)) => {
            if let Some(images) = images.upgrade() { return Ok(images); }
        }
        None => {}
    }
    let images: LibraryImages = Arc::new(load(catalog, resource, file)?.into_iter()
        .map(|(id, image)| (id, Arc::new(image))).collect());
    let entry = if retain_session_payload() {
        CachedLibrary::Session(images.clone())
    } else {
        CachedLibrary::Live(Arc::downgrade(&images))
    };
    resident.insert(key, entry);
    Ok(images)
}
