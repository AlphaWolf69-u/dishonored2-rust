//! Understood logical buffer-view cache in FUN_1402d38c0.
//!
//! This resolves and caches view descriptors, not GPU allocation or clearing.
//! FUN_1400998b0 is a no-op. Native allocator/critical-section ABI and the
//! owning GPU buffer lifecycle are not reproduced by this safe value layer.
//!
//! Pool allocation is now proved: FUN_140371750 creates1,152,000 four-byte
//! links and a separate four-byte UAV counter (chunk_000126.c:4176-4269).
//! FUN_140370860 flags3 creates a GPU-only uninitialized pool, with default
//! usage, bind flags0x28 and stride4. FUN_140351780 schedules the zero-UAV
//! descriptor initialized by FUN_1403772d0 for ctx+0xc798; the equivalent
//! counter reset now lives in tiled_lighting_gpu::NativeTlfBuffers::begin_frame.
//! List-head initialization and live depth-pass ownership remain separate.
//! Do not infer D3D11 clears
//! from an arbitrary +0x198 virtual call: inspected 140348bb0 is monitor-mode
//! selection (global1422fcd70), not a UAV clear. Its full CVar side effects,
//! monitor-interface layout and error contract remain unmapped; raw capture
//! cfc490902e7bbaa49dad25ae.json preserves the complete 102-line body.
//! Other +0x198 snippets 140726da0/140727660/1407272b0/14003f450/
//! 1400459a0/14043cc50/14113bf30 did not establish a GPU-context receiver;
//! only callsite windows were inspected, not complete function semantics.

use std::collections::BTreeMap;
use crate::texture::image_bits_per_pixel;

/// FUN_1402fc580/1402fc600/140331740/1402f82a0 specialize this recurrence
/// for 8/11/12/24 bytes. MOVSX at 140331740 and 1403317ca proves SIGNED
/// bytes; ordinary unsigned-byte FNV-1a is not equivalent for bytes >=128.
pub const fn resource_hash_bytes(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c9dc5u32;
    let mut index = 0;
    while index < bytes.len() {
        hash = (hash ^ bytes[index] as i8 as i32 as u32).wrapping_mul(0x1000193);
        index += 1;
    }
    hash
}

/// Original named-resource callsites also hash their terminal NUL.
/// For example 351780 hashes `lightNodePool`'s last byte, then multiplies
/// by 0x26027a69 (=0x1000193 squared), including the zero terminator.
pub const fn resource_name_key(name: &str) -> u32 {
    resource_hash_bytes(name.as_bytes()).wrapping_mul(0x1000193)
}

/// Typed render-task properties used by FUN_14034a6b0. Registration replaces
/// the first matching name in place; new names retain registration order.
/// Rust owns the values instead of reproducing the native shared-pointer ABI.
#[derive(Debug)]
pub struct RenderTaskProperties<T> {
    entries: Vec<(u32, T)>,
}

impl<T> Default for RenderTaskProperties<T> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<T> RenderTaskProperties<T> {
    pub fn register(&mut self, name: u32, value: T) {
        if let Some((_, current)) = self.entries.iter_mut().find(|(key, _)| *key == name) {
            *current = value;
        } else {
            self.entries.push((name, value));
        }
    }

    pub fn get(&self, name: u32) -> Option<&T> {
        self.entries.iter().find(|(key, _)| *key == name).map(|(_, value)| value)
    }
}

/// FUN_14034d140 fans the same registration out to every render-task descriptor.
/// Only the logical registration path is ported, not native allocator failure
/// or shared-pointer control-block/destructor callbacks.
pub fn register_render_task_property<T: Clone>(tasks: &mut [RenderTaskProperties<T>], name: u32, value: T) {
    for task in tasks {
        task.register(name, value.clone());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferView {
    pub first_element: u32,
    pub element_count: u32,
    pub format_word: u32,
}

/// Native lossy key: do not replace it with a collision-free tuple key.
pub fn buffer_view_key(format_word: u32, first: u32, count: u32) -> u32 {
    let spread = |value: u32| {
        let high = (value as i32 >> 4) as u32;
        (high.wrapping_shl(10) ^ high) & 0x000f_fc00
    };
    ((format_word >> 13) & 255) ^ ((count & 15) << 4 | first & 15
        | spread(count).wrapping_shl(12) | spread(first).wrapping_shl(2))
}

#[derive(Debug, Default)]
pub struct BufferViewCache {
    views: BTreeMap<u32, Box<BufferView>>,
}

impl BufferViewCache {
    pub fn get(
        &mut self, byte_width: u32, default_element_count: u32,
        format_word: u32, first: u32, requested_count: u32,
    ) -> Result<&BufferView, String> {
        let available = if format_word & 0x001f_e000 != 0 {
            let bpp = image_bits_per_pixel(format_word);
            if bpp == 0 { return Err("native typed buffer view has zero bits per pixel".into()); }
            byte_width.wrapping_shl(3) / bpp
        } else { default_element_count };
        let count = if requested_count == 0 { available } else { requested_count };
        let key = buffer_view_key(format_word,first,count);
        // Only native key equality matters, even when source fields differ.
        Ok(self.views.entry(key).or_insert_with(|| Box::new(BufferView {
            first_element:first,element_count:count,format_word,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_task_property_registration_replaces_in_place_and_fans_out() {
        let mut tasks = [RenderTaskProperties::<u64>::default(), RenderTaskProperties::default()];
        let packets = resource_name_key("cpu_lightToPutInTiles");
        let constants = resource_name_key("cpu_lightToPutInTiles_cb");
        register_render_task_property(&mut tasks, packets, 11);
        register_render_task_property(&mut tasks, constants, 22);
        register_render_task_property(&mut tasks, packets, 33);
        for task in tasks {
            assert_eq!(task.entries, [(packets, 33), (constants, 22)]);
            assert_eq!(task.get(packets), Some(&33));
            assert_eq!(task.get(0), None);
        }
        register_render_task_property::<u64>(&mut [], packets, 44);
    }

    #[test]
    fn resource_hash_specializations_match_original_machine_code() {
        let mut checked = 0;
        for line in include_str!("../analysis/original_resource_name_hash_cases.txt").lines() {
            let fields: Vec<_> = line.split_whitespace().collect();
            let bytes: Vec<u8> = fields[1].as_bytes().chunks_exact(2)
                .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(),16).unwrap()).collect();
            let expected = u32::from_str_radix(fields[2],16).unwrap();
            assert_eq!(resource_hash_bytes(&bytes),expected,"{line}");
            checked += 1;
        }
        assert_eq!(checked,512);
        assert_ne!(resource_hash_bytes(&[0x80]),(0x811c9dc5u32 ^ 0x80).wrapping_mul(0x1000193));
        for name in ["lightNodePool","lightNodePoolNextFreeEntry","perTileLightListHeads"] {
            let mut terminated=name.as_bytes().to_vec();terminated.push(0);
            assert_eq!(resource_name_key(name),resource_hash_bytes(&terminated));
        }
    }

    #[test]
    fn buffer_views_match_original_cache_fields_and_identity() {
        let mut cache=BufferViewCache::default();
        let mut case=None;let mut identities=BTreeMap::new();let mut checked=0;
        for line in include_str!("../analysis/original_buffer_view_cases.txt").lines() {
            let f:Vec<u32>=line.split_whitespace().map(|v|v.parse().unwrap()).collect();
            if case!=Some(f[0]) {cache=BufferViewCache::default();identities.clear();case=Some(f[0]);}
            let view=cache.get(f[1],f[2],f[3],f[4],f[5]).unwrap();
            assert_eq!((view.first_element,view.element_count,view.format_word),(f[6],f[7],f[8]),"{line}");
            let address=view as *const BufferView as usize;
            let repeated=identities.contains_key(&address);
            let next=identities.len() as u32;
            assert_eq!(*identities.entry(address).or_insert(next),f[9],"{line}");
            assert_eq!(u32::from(repeated),f[10],"{line}");checked+=1;
        }
        assert_eq!(checked,1350);
    }

    #[test]
    fn original_buffer_view_zero_count_and_cached_identity() {
        let mut cache=BufferViewCache::default();
        let view=cache.get(4608000,1152000,0,0,0).unwrap();
        assert_eq!(view.element_count,1152000);
        let address=view as *const BufferView;
        assert_eq!(cache.get(1,1152000,0,0,0).unwrap() as *const BufferView,address);
        // Native R32 family=3, converted format=42 (nonzero format field).
        let word=3 | (42<<13);
        assert_eq!(cache.get(400,1,word,2,0).unwrap().element_count,100);
        assert_eq!(cache.get(400,1,word,2,7).unwrap().element_count,7);
        assert!(cache.get(400,1,42<<13,2,0).is_err());
    }

    #[test]
    fn native_buffer_key_keeps_original_aliasing() {
        let mut cache=BufferViewCache::default();
        // Top index bits are discarded by the native key construction.
        let first=cache.get(4,1,0,0,1).unwrap() as *const BufferView;
        assert_eq!(buffer_view_key(0,0,1),buffer_view_key(0,0x8000_0000,1));
        assert_eq!(cache.get(4,1,0,0x8000_0000,1).unwrap() as *const BufferView,first);
        assert_eq!(cache.get(4,1,0,0x8000_0000,1).unwrap().first_element,0);
    }
}
