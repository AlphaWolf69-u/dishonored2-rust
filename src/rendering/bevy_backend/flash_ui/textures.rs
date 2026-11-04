//! Host image-cache leases for GDraw's bounded texture owner. The installed
//! limits come from 140034ba0 -> 1402df160/1402e3460 (cache kind 1).
use super::TextureKey;
use bevy::render::render_resource::{Texture, TextureView};
use std::collections::HashMap;

const MAX_ENTRIES: usize = 0x800;
const MAX_PAYLOAD_BYTES: u64 = 0x1428000;
type Image = (Texture, TextureView);

struct Entry {
    image: Image,
    bytes: u64,
    used: u64,
    ordinal: u64,
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
            true
        } else {
            false
        }
    }
    pub fn insert(&mut self, key: TextureKey, image: Image) {
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
            },
        ) {
            self.bytes -= previous.bytes;
        }
        if self.trace {
            self.peak_entries = self.peak_entries.max(self.entries.len());
            self.peak_bytes = self.peak_bytes.max(self.bytes);
        }
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
            eprintln!("MEMORY UI texture cache: phase={phase} entries={} payload_bytes={} peak_entries={} peak_payload_bytes={} pressure_evictions={} active_over_budget={} unbounded={}",
                self.entries.len(), self.bytes, self.peak_entries, self.peak_bytes,
                self.evictions, self.over_budget(), self.unbounded);
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
