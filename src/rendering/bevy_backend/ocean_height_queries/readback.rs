//! 3030f0's four staging buffers and 304c40's selected-slot wait/map/copy/unmap.
use super::*;
use crate::rendering::ocean::OceanQueryQueue;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MappedReadback {
    pending: bool,
    result: Option<Result<Vec<[f32; 4]>, String>>,
}

struct ReadbackSlot {
    buffer: Buffer,
    encoded: bool,
    wait: Option<PollType>,
    // Only actually copied values are initialized/exposed. A shorter update
    // retains the previous tail, as in the source's fixed-capacity CPU array.
    completed: Vec<[f32; 4]>,
    mapped: Arc<Mutex<MappedReadback>>,
    count: usize,
}

pub struct OceanQueryReadbacks {
    slots: [ReadbackSlot; 4],
}

impl OceanQueryReadbacks {
    pub fn new(device: &RenderDevice) -> Self {
        Self {
            slots: std::array::from_fn(|_| ReadbackSlot {
                buffer: device.create_buffer(&BufferDescriptor {
                    label: Some("ocean query staging"),
                    size: 16384,
                    usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                encoded: false,
                wait: None,
                completed: Vec::with_capacity(OCEAN_QUERY_CAPACITY),
                mapped: Arc::default(),
                count: 0,
            }),
        }
    }

    /// Record only for a nonempty evaluation dispatch; copy the source's entire
    /// 16384-byte result resource. This does not imply queue submission/readiness.
    pub(super) fn record_copy(
        &mut self,
        encoder: &mut CommandEncoder,
        evaluations: &Buffer,
        count: u32,
        ring: usize,
    ) -> Result<(), String> {
        let slot = self
            .slots
            .get_mut(ring)
            .ok_or("ocean query ring is outside four slots")?;
        if count > 0 {
            if slot
                .mapped
                .lock()
                .map_err(|_| "ocean readback poisoned")?
                .pending
            {
                return Err("ocean query ring is still mapped".into());
            }
            encoder.copy_buffer_to_buffer(evaluations, 0, &slot.buffer, 0, 16384);
            slot.encoded = true;
            slot.count = count as usize;
        }
        Ok(())
    }

    /// Call AFTER submitting the command batch containing the recorded copies.
    /// The real submission is carried inside Bevy's already-exported PollType;
    /// no added wgpu dependency or wait for unrelated later work is needed.
    pub fn mark_submitted(&mut self, wait: PollType) -> Result<(), String> {
        if !matches!(
            &wait,
            PollType::Wait {
                submission_index: Some(_),
                ..
            }
        ) {
            return Err("ocean readback requires the actual copy submission".into());
        }
        for slot in &mut self.slots {
            if slot.encoded {
                slot.wait = Some(wait.clone());
                slot.encoded = false;
            }
        }
        Ok(())
    }

    /// 304c40 reads exactly the current ring when enabled and its count>0.
    /// It waits for that buffer's recorded submission, not an arbitrary ready
    /// slot. Caller supplies the source renderer's current ring and activity.
    pub fn read_current(
        &mut self,
        device: &RenderDevice,
        enabled: bool,
        ring: usize,
        queries: &OceanQueryQueue,
    ) -> Result<bool, String> {
        let count = queries
            .submitted_count(ring)
            .ok_or("ocean query ring is outside four slots")?;
        if !enabled || count == 0 {
            return Ok(false);
        }
        let slot = &mut self.slots[ring];
        if slot.encoded {
            return Err("ocean staging copy has not been submitted".into());
        }
        let wait = slot
            .wait
            .clone()
            .ok_or("ocean staging buffer has no submitted evaluation")?;
        let (tx, rx) = std::sync::mpsc::channel();
        slot.buffer
            .slice(..)
            .map_async(MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        if let Err(error) = device.poll(wait) {
            slot.buffer.unmap();
            return Err(format!("ocean query wait failed: {error}"));
        }
        match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                slot.buffer.unmap();
                return Err(format!("ocean query map failed: {error}"));
            }
            Err(error) => {
                slot.buffer.unmap();
                return Err(format!("ocean query map callback failed: {error}"));
            }
        }
        let mapped = slot.buffer.slice(..).get_mapped_range();
        if slot.completed.len() < count {
            slot.completed.resize(count, [0.0; 4]);
        }
        for (value, bytes) in slot.completed[..count]
            .iter_mut()
            .zip(mapped.chunks_exact(16))
        {
            for (channel, bytes) in value.iter_mut().zip(bytes.chunks_exact(4)) {
                *channel = f32::from_le_bytes(bytes.try_into().unwrap());
            }
        }
        drop(mapped);
        slot.buffer.unmap();
        Ok(true)
    }

    pub fn completed_rings(&self) -> [&[[f32; 4]]; 4] {
        self.slots.each_ref().map(|slot| slot.completed.as_slice())
    }
    /// Host submission bridge: Bevy submits the graph before Cleanup. Map the
    /// persistent slots there, without adding another queue submission.
    pub fn map_submitted(&mut self) -> Result<(), String> {
        for slot in &mut self.slots {
            if !slot.encoded {
                continue;
            }
            slot.encoded = false;
            let mut state = slot.mapped.lock().map_err(|_| "ocean readback poisoned")?;
            if state.pending {
                return Err("ocean staging buffer already mapped".into());
            }
            state.pending = true;
            state.result = None;
            drop(state);
            let state = slot.mapped.clone();
            let buffer = slot.buffer.clone();
            let count = slot.count;
            slot.buffer
                .slice(..)
                .map_async(MapMode::Read, move |result| {
                    let result = result
                        .map_err(|e| format!("ocean staging map failed: {e}"))
                        .map(|_| {
                            let mapped = buffer.slice(..).get_mapped_range();
                            let values = mapped
                                .chunks_exact(16)
                                .take(count)
                                .map(|v| {
                                    std::array::from_fn(|c| {
                                        f32::from_le_bytes(v[c * 4..c * 4 + 4].try_into().unwrap())
                                    })
                                })
                                .collect();
                            drop(mapped);
                            values
                        });
                    buffer.unmap();
                    if let Ok(mut state) = state.lock() {
                        state.pending = false;
                        state.result = Some(result);
                    }
                });
        }
        Ok(())
    }
    /// 304c40 consumes the selected ring, rather than whichever result happens
    /// to finish first. Its rare wait stays on the render thread.
    pub fn read_current_mapped(
        &mut self,
        device: &RenderDevice,
        ring: usize,
        queries: &OceanQueryQueue,
    ) -> Result<bool, String> {
        let count = queries
            .submitted_count(ring)
            .ok_or("ocean ring out of range")?;
        if count == 0 {
            return Ok(false);
        }
        let slot = &mut self.slots[ring];
        if slot.encoded {
            return Err("ocean copy has not reached queue submission".into());
        }
        let pending = slot
            .mapped
            .lock()
            .map_err(|_| "ocean readback poisoned")?
            .pending;
        // poll may execute the map callback synchronously. Release its mutex
        // before waiting, otherwise the render thread deadlocks in that callback.
        if pending {
            device
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(30)),
                })
                .map_err(|e| format!("ocean staging wait failed: {e}"))?;
        }
        let Some(values) = slot
            .mapped
            .lock()
            .map_err(|_| "ocean readback poisoned")?
            .result
            .take()
        else {
            return Ok(false);
        };
        let values = values?;
        if values.len() != count {
            return Err("ocean ring result count differs from its submission".into());
        }
        if slot.completed.len() < count {
            slot.completed.resize(count, [0.; 4]);
        }
        slot.completed[..count].copy_from_slice(&values);
        Ok(true)
    }
    pub fn buffer_bytes(&self) -> u64 {
        self.slots.iter().map(|slot| slot.buffer.size()).sum()
    }
    #[cfg(test)]
    pub(super) fn buffer_ids(&self) -> [BufferId; 4] {
        self.slots.each_ref().map(|slot| slot.buffer.id())
    }
}
