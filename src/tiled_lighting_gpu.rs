//! Reusable native TLF GPU buffers; pass ownership and dispatch remain external.
use bevy::render::{
    render_resource::{Buffer, BufferDescriptor, BufferUsages, CommandEncoder},
    renderer::{RenderDevice, RenderQueue},
};

use crate::render_resource::resource_name_key;
use crate::tiled_lighting::{
    LIGHT_BUFFER_CAPACITY, LIGHT_NODE_COUNTER_BYTES, LIGHT_NODE_POOL_CAPACITY,
};

/// Native FUN_140371750 allocations: 448 × 48-byte cull packets, 1,152,000
/// four-byte pool links, and a separate four-byte counter. FUN_140351780
/// clears the counter once per frame before normal and scatter passes.
/// The caller must separately initialize each pass's heads to END_OF_LIGHT_LIST;
/// cull_lights exchanges/appends heads, it does not clear them. Stale pool links
/// are unreachable only after both counter and head initialization.
/// Light/shadow payload owners and material/shader pipeline integration remain
/// outside this resource.
pub struct NativeTlfBuffers {
    pub packets: Buffer,
    pub counter: Buffer,
    pub pool: Buffer,
}

impl NativeTlfBuffers {
    pub fn new(device: &RenderDevice) -> Self {
        Self {
            packets: device.create_buffer(&BufferDescriptor {
                label: Some("native TLF cull packets"),
                size: (LIGHT_BUFFER_CAPACITY * 48) as u64,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            counter: device.create_buffer(&BufferDescriptor {
                label: Some("native TLF shared node counter"),
                size: LIGHT_NODE_COUNTER_BYTES,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pool: device.create_buffer(&BufferDescriptor {
                label: Some("native TLF light-node pool"),
                size: (LIGHT_NODE_POOL_CAPACITY * 4) as u64,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
        }
    }

    /// Resolve the three named buffer inputs used by FUN_140351780's
    /// TLF callback, including the original terminal-NUL resource hash.
    pub fn resource(&self, key: u32) -> Option<&Buffer> {
        match key {
            k if k == resource_name_key("lightToPutInTiles") => Some(&self.packets),
            k if k == resource_name_key("lightNodePoolNextFreeEntry") => Some(&self.counter),
            k if k == resource_name_key("lightNodePool") => Some(&self.pool),
            _ => None,
        }
    }

    /// Upload whole 48-byte packet records; inputs outside the native capacity
    /// or packet stride are rejected rather than truncated.
    pub fn upload_cull_packets(&self, queue: &RenderQueue, words: &[u32]) -> Result<(), String> {
        if words.len() % 12 != 0 {
            return Err("native TLF cull packets must contain 12 words each".into());
        }
        if words.len() / 12 > LIGHT_BUFFER_CAPACITY {
            return Err("native TLF cull packet count exceeds 448".into());
        }
        let bytes = words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        if !bytes.is_empty() {
            queue.write_buffer(&self.packets, 0, &bytes);
        }
        Ok(())
    }

    /// Match FUN_140351780: reset the separate allocation counter once before
    /// both normal and scattering culls. Do not clear the pool or reset between passes.
    pub fn begin_frame(&self, encoder: &mut CommandEncoder) {
        encoder.clear_buffer(&self.counter, 0, None);
    }
}
