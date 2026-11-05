//! Host image-cache leases for GDraw's bounded texture owner. The installed
//! limits come from 140034ba0 -> 1402df160/1402e3460 (cache kind 1).
use super::TextureKey;
use bevy::render::render_resource::{Texture, TextureView};
use std::collections::HashMap;
use std::sync::{Arc, Weak};

type BitmapMap = HashMap<crate::flash::player::CharRef, Arc<crate::flash::bitmap_resources::Image>>;

const MAX_ENTRIES: usize = 0x800;
const MAX_PAYLOAD_BYTES: u64 = 0x1428000;
type Image = (Texture, TextureView);

struct Entry {
    image: Image,
    bytes: u64,
    used: u64,
    ordinal: u64,
    bitmap_owners: Vec<Weak<BitmapMap>>,
    bitmap_image: Option<Weak<crate::flash::bitmap_resources::Image>>,
    glyph_owner: Option<Weak<crate::flash::glyph_cache::Page>>,
}

pub(super) struct UiTextures {
    entries: HashMap<TextureKey, Entry>,
    bytes: u64,
    frame: u64,
    next_ordinal: u64,
    pressure: Vec<((u64, u64), u64)>,
    evictions: u64,
    peak_entries: usize,
    peak_bytes: u64,
    bitmap_uploads: u64,
    bitmap_reuses: u64,
    unowned_retirements: u64,
    key_build_time: std::time::Duration,
    key_build_frames: u64,
    owner_scan_time: std::time::Duration,
    owner_scans: u64,
    owner_track_time: std::time::Duration,
    owner_track_frames: u64,
    trace: bool,
    unbounded: bool,
}
impl Default for UiTextures {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            bytes: 0,
            frame: 0,
            next_ordinal: 0,
            pressure: Vec::new(),
            evictions: 0,
            peak_entries: 0,
            peak_bytes: 0,
            bitmap_uploads: 0,
            bitmap_reuses: 0,
            unowned_retirements: 0,
            key_build_time: std::time::Duration::ZERO,
            key_build_frames: 0,
            owner_scan_time: std::time::Duration::ZERO,
            owner_scans: 0,
            owner_track_time: std::time::Duration::ZERO,
            owner_track_frames: 0,
            trace: std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"),
            unbounded: std::env::var("DH2_DEBUG_RETAIN_UI_TEXTURES").as_deref() == Ok("1"),
        }
    }
}
impl UiTextures {
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn tracing(&self) -> bool {
        self.trace
    }
    pub fn get(&self, key: &TextureKey) -> Option<&Image> {
        self.entries.get(key).map(|entry| &entry.image)
    }
    pub fn keys(&self) -> impl Iterator<Item = &TextureKey> {
        self.entries.keys()
    }
    pub fn touch(&mut self, key: &TextureKey) -> bool {
        if self.unbounded {
            return self.entries.contains_key(key);
        }
        if let Some(entry) = self.entries.get_mut(key) {
            entry.used = self.frame;
            if self.trace && matches!(key, TextureKey::Bitmap(_)) { self.bitmap_reuses += 1; }
            true
        } else {
            false
        }
    }
    pub fn insert(&mut self, key: TextureKey, image: Image) {
        if self.trace && matches!(key, TextureKey::Bitmap(_)) { self.bitmap_uploads += 1; }
        let bytes = if matches!(key, TextureKey::Game(_)) {
            0
        } else {
            payload_bytes(&image.0)
        };
        self.bytes += bytes;
        let ordinal = self.next_ordinal;
        self.next_ordinal = self.next_ordinal.wrapping_add(1);
        if let Some(previous) = self.entries.insert(
            key,
            Entry {
                image,
                bytes,
                used: self.frame,
                ordinal,
                bitmap_owners: Vec::new(),
                bitmap_image: None,
                glyph_owner: None,
            },
        ) {
            self.bytes -= previous.bytes;
        }
        if self.trace {
            self.peak_entries = self.peak_entries.max(self.entries.len());
            self.peak_bytes = self.peak_bytes.max(self.bytes);
        }
    }
    /// A GPU bitmap is shared by every live movie that references the same
    /// immutable image. Weak maps do not retain the movie or its assets.
    pub fn track_bitmap(&mut self, key: &TextureKey, images: &Arc<BitmapMap>, image: &Arc<crate::flash::bitmap_resources::Image>) {
        let identity = Arc::downgrade(image);
        // Keep a weak allocation reference as well as the movie-map leases:
        // this prevents an address from being recycled before pruning and
        // lets us reject an unexpected identity mismatch safely.
        let mismatch = self.entries.get(key).and_then(|entry| entry.bitmap_image.as_ref())
            .is_some_and(|existing| !existing.ptr_eq(&identity));
        if mismatch { self.remove(key); return; }
        let Some(entry) = self.entries.get_mut(key) else { return };
        entry.bitmap_image = Some(identity);
        entry.bitmap_owners.retain(|owner| owner.strong_count() != 0);
        let owner = Arc::downgrade(images);
        if !entry.bitmap_owners.iter().any(|existing| existing.ptr_eq(&owner)) {
            entry.bitmap_owners.push(owner);
        }
    }
    pub fn track_glyph(&mut self, key: &TextureKey, page: &Arc<crate::flash::glyph_cache::Page>) {
        if let Some(entry) = self.entries.get_mut(key) { entry.glyph_owner = Some(Arc::downgrade(page)); }
    }
    /// The session bitmap registry may outlive movies, so image Arc counts
    /// alone do not establish that a GPU copy is still needed.
    pub fn retire_unowned(&mut self, incoming_pages: &[Arc<crate::flash::glyph_cache::Page>]) {
        let started = self.trace.then(std::time::Instant::now);
        let mut removed = 0;
        let mut count = 0;
        self.entries.retain(|key, entry| {
            let keep = match key {
                TextureKey::Bitmap(_) => {
                    entry.bitmap_owners.retain(|owner| owner.strong_count() != 0);
                    !entry.bitmap_owners.is_empty()
                }
                TextureKey::Glyphs(id, _) => incoming_pages.iter().any(|page| page.id == *id)
                    || entry.glyph_owner.as_ref().is_some_and(|owner| owner.strong_count() != 0),
                _ => true,
            };
            if !keep { removed += entry.bytes; count += 1; }
            keep
        });
        self.bytes -= removed;
        if self.trace {
            self.unowned_retirements += count;
            if count != 0 { eprintln!("MEMORY UI unowned textures retired: count={count} bytes={removed}"); }
        }
        if let Some(started) = started { self.owner_scans += 1; self.owner_scan_time += started.elapsed(); }
    }
    pub fn record_key_build(&mut self, duration: std::time::Duration) {
        if self.trace { self.key_build_frames += 1; self.key_build_time += duration; }
    }
    pub fn record_owner_tracking(&mut self, duration: std::time::Duration) {
        if self.trace { self.owner_track_frames += 1; self.owner_track_time += duration; }
    }
    pub fn remove(&mut self, key: &TextureKey) -> Option<Image> {
        let entry = self.entries.remove(key)?;
        self.bytes -= entry.bytes;
        Some(entry.image)
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&TextureKey, &Image) -> bool) {
        let mut removed = 0;
        self.entries.retain(|key, entry| {
            if keep(key, &entry.image) {
                true
            } else {
                removed += entry.bytes;
                false
            }
        });
        self.bytes -= removed;
    }
    fn over_budget(&self) -> bool {
        self.entries.len() > MAX_ENTRIES || self.bytes > MAX_PAYLOAD_BYTES
    }
    /// 1402e6540 / 1402e67e0 retire old cache entries under pressure. Host:
    /// retain the whole current draw list instead of flushing/waiting for GPU
    /// fences inside UI rendering; submitted command buffers own their leases.
    pub fn trim(&mut self) {
        if self.unbounded || !self.over_budget() {
            return;
        }
        self.pressure.clear();
        self.pressure.extend(
            self.entries
                .iter()
                .filter(|(key, entry)| {
                    entry.used != self.frame && !matches!(key, TextureKey::White)
                })
                .map(|(_, entry)| ((entry.used, entry.ordinal), entry.bytes)),
        );
        self.pressure.sort_unstable_by_key(|(order, _)| *order);
        let mut count = self.entries.len();
        let mut bytes = self.bytes;
        let mut cutoff = None;
        for &(order, charge) in &self.pressure {
            if count <= MAX_ENTRIES && bytes <= MAX_PAYLOAD_BYTES {
                break;
            }
            count -= 1;
            bytes -= charge;
            cutoff = Some(order);
        }
        if let Some(cutoff) = cutoff {
            let frame = self.frame;
            let mut removed_bytes = 0;
            let mut removed_count = 0;
            self.entries.retain(|key, entry| {
                let retire = entry.used != frame
                    && !matches!(key, TextureKey::White)
                    && (entry.used, entry.ordinal) <= cutoff;
                if retire {
                    removed_bytes += entry.bytes;
                    removed_count += 1;
                }
                !retire
            });
            self.bytes -= removed_bytes;
            self.evictions += removed_count;
        }
        self.pressure.clear();
    }
    pub fn report(&self, phase: &str) {
        if self.trace {
            eprintln!("MEMORY UI texture cache: phase={phase} entries={} payload_bytes={} peak_entries={} peak_payload_bytes={} pressure_evictions={} unowned_retirements={} bitmap_uploads={} bitmap_batch_reuses={} key_build_frames={} key_build_us={:.1} owner_scans={} owner_scan_us={:.1} owner_track_frames={} owner_track_us={:.1} active_over_budget={} unbounded={}",
                self.entries.len(), self.bytes, self.peak_entries, self.peak_bytes,
                self.evictions, self.unowned_retirements, self.bitmap_uploads, self.bitmap_reuses,
                self.key_build_frames, self.key_build_time.as_secs_f64() * 1e6,
                self.owner_scans, self.owner_scan_time.as_secs_f64() * 1e6,
                self.owner_track_frames, self.owner_track_time.as_secs_f64() * 1e6,
                self.over_budget(), self.unbounded);
        }
    }
}
impl Drop for UiTextures {
    fn drop(&mut self) {
        self.report("release");
    }
}

/// Nominal image storage, as charged by 1402e5350. Backend block padding is
/// included; physical allocation granularity/driver heaps are reported separately.
fn payload_bytes(texture: &Texture) -> u64 {
    let format = texture.format();
    let (block_width, block_height) = format.block_dimensions();
    let block_bytes = format
        .block_copy_size(None)
        .expect("UI image has a copyable colour format");
    (0..texture.mip_level_count())
        .map(|mip| {
            u64::from((texture.width() >> mip).max(1).div_ceil(block_width))
                * u64::from((texture.height() >> mip).max(1).div_ceil(block_height))
                * u64::from(block_bytes)
        })
        .sum()
}
