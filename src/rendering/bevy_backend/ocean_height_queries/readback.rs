//! 3030f0's four staging buffers and 304c40's selected-slot wait/map/copy/unmap.
use super::*;
use crate::rendering::ocean::OceanQueryQueue;
use std::sync::{Arc, Mutex};

#[derive(Default)]
enum MapState {
    #[default]
    Idle,
    Pending,
    Ready(Result<(), BufferAsyncError>),
}

struct ReadbackSlot {
    buffer: Buffer,
    encoded: bool,
    wait: Option<PollType>,
    // Only actually copied values are initialized/exposed. A shorter update
    // retains the previous tail, as in the source's fixed-capacity CPU array.
    completed: Vec<[f32; 4]>,
    mapped: Arc<Mutex<MapState>>,
    count: usize,
}

impl ReadbackSlot {
    fn copy_mapped(&mut self, count: usize) {
        let mapped = self.buffer.slice(..).get_mapped_range();
        if self.completed.len() < count {
            self.completed.resize(count, [0.0; 4]);
        }
        for (value, bytes) in self.completed[..count].iter_mut().zip(mapped.chunks_exact(16)) {
            for (channel, bytes) in value.iter_mut().zip(bytes.chunks_exact(4)) {
                *channel = f32::from_le_bytes(bytes.try_into().unwrap());
            }
        }
        drop(mapped);
        self.buffer.unmap();
    }
}

impl Drop for ReadbackSlot {
    fn drop(&mut self) {
        let state = std::mem::take(&mut *self.mapped.lock().unwrap_or_else(|error| error.into_inner()));
        super::super::readback_lifecycle::owner_dropped(super::super::readback_lifecycle::Kind::OceanHeight,&self.mapped,
            matches!(state,MapState::Pending),matches!(state,MapState::Ready(_)));
        if matches!(state, MapState::Pending | MapState::Ready(Ok(()))) {
            self.buffer.unmap();
        }
    }
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
                mapped: {
                    let mapped: Arc<Mutex<MapState>>=Arc::default();
                    super::super::readback_lifecycle::register(super::super::readback_lifecycle::Kind::OceanHeight,&mapped);
                    mapped
                },
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
            let mut state = slot.mapped.lock().map_err(|_| "ocean readback poisoned")?;
            if matches!(*state, MapState::Pending) {
                return Err("ocean query ring is still mapped".into());
            }
            // The selected ring normally consumes its map before reuse. A
            // skipped read must still release the completed mapping.
            let previous = std::mem::take(&mut *state);
            drop(state);
            if matches!(previous, MapState::Ready(Ok(()))) {
                slot.buffer.unmap();
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
                return Err(format!("ocean query map failed: {error}"));
            }
            Err(error) => {
                slot.buffer.unmap();
                return Err(format!("ocean query map callback failed: {error}"));
            }
        }
        slot.copy_mapped(count);
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
            if !matches!(*state, MapState::Idle) {
                return Err("ocean staging buffer already mapped".into());
            }
            *state = MapState::Pending;
            drop(state);
            let state = slot.mapped.clone();
            super::super::readback_lifecycle::map_started(super::super::readback_lifecycle::Kind::OceanHeight,&state);
            slot.buffer
                .slice(..)
                .map_async(MapMode::Read, move |result| {
                    super::super::readback_lifecycle::callback_completed(super::super::readback_lifecycle::Kind::OceanHeight,&state,result.is_ok());
                    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
                    if matches!(*state, MapState::Pending) {
                        *state = MapState::Ready(result);
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
        let pending = matches!(
            *slot.mapped.lock().map_err(|_| "ocean readback poisoned")?,
            MapState::Pending
        );
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
        let result = {
            let mut state = slot.mapped.lock().map_err(|_| "ocean readback poisoned")?;
            if !matches!(*state, MapState::Ready(_)) {
                return Ok(false);
            }
            std::mem::take(&mut *state)
        };
        if let MapState::Ready(Err(error)) = result {
            return Err(format!("ocean staging map failed: {error}"));
        }
        if slot.count != count {
            slot.buffer.unmap();
            return Err("ocean ring result count differs from its submission".into());
        }
        slot.copy_mapped(count);
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

/// Windowless observation of the real staging owner and its queue boundary.
pub(crate) fn observe_readback(device: &RenderDevice, queue: &RenderQueue) -> Result<(), String> {
    let source = device.create_buffer(&BufferDescriptor {
        label: Some("observed ocean height source"),
        size: 16384,
        usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let samples: [[f32; 4]; 5] = std::array::from_fn(|i| [i as f32, -0.25, 0.5, 1.0]);
    let bytes: Vec<_> = samples.iter().flatten().flat_map(|value| value.to_le_bytes()).collect();
    queue.write_buffer(&source, 0, &bytes);
    let mut readbacks = OceanQueryReadbacks::new(device);
    let mut requests = OceanQueryQueue::new();
    let count = samples.len() as u32;
    let submit = |readbacks: &mut OceanQueryReadbacks, ring| -> Result<_, String> {
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        readbacks.record_copy(&mut encoder, &source, count, ring)?;
        Ok(PollType::Wait { submission_index: Some(queue.submit([encoder.finish()])), timeout: None })
    };
    requests.register(3, &[[0.0; 2]; 5])?;
    requests.record_submission(0)?;
    let wait = submit(&mut readbacks, 0)?;
    readbacks.mark_submitted(wait)?;
    readbacks.read_current(device, true, 0, &requests)?;
    requests.register(0, &[[0.0; 2]; 5])?;
    requests.record_submission(1)?;
    submit(&mut readbacks, 1)?;
    readbacks.map_submitted()?;
    let read = readbacks.read_current_mapped(device, 1, &requests)?;
    let rings = readbacks.completed_rings();
    eprintln!("OCEAN_READBACK_SELECTED read={read} samples={} matches={} staging_bytes={}",
        rings[1].len(), rings[0] == rings[1] && rings[1] == samples, readbacks.buffer_bytes());
    let wait = submit(&mut readbacks, 2)?;
    readbacks.map_submitted()?;
    let pending = matches!(*readbacks.slots[2].mapped.lock().unwrap(), MapState::Pending);
    let callback = Arc::downgrade(&readbacks.slots[2].mapped);
    drop(readbacks);
    device.poll(wait).map_err(|error| error.to_string())?;
    eprintln!("OCEAN_READBACK_RELEASE pending={pending} callback_leases={}", callback.strong_count());
    Ok(())
}
