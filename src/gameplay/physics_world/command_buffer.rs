//! Expandable command storage (fa400/fa4f0/fa6a0/fa6d0) and089eb0 dispatch.
use crate::storage::PointerList;
use std::{
    cell::UnsafeCell,
    sync::{
        atomic::{AtomicI32, AtomicPtr, Ordering},
        Arc,
    },
};

struct Chunk {
    index: usize,
    data: Box<[UnsafeCell<u8>]>,
    reserved: AtomicI32,
    records: AtomicI32,
}
// SAFETY: byte storage is private. Producers write only their disjoint atomic
// reservations; packet readers require exclusive access to CommandBuffer.
unsafe impl Sync for Chunk {}
struct Storage {
    order: PointerList,
    // Arc keeps headers and byte allocations stable while producers grow the
    // pointer list. Blocks remain alive until an exclusive reset or drop.
    chunks: Vec<Arc<Chunk>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandLocation {
    pub chunk: usize,
    pub offset: usize,
}

pub struct CommandBuffer {
    chunk_size: i32,
    memory_tag: u32,
    lock: AtomicI32,
    current: AtomicPtr<Chunk>,
    storage: UnsafeCell<Storage>,
}

// SAFETY: shared producers access chunk headers only through atomics and write
// disjoint byte ranges returned by fetch_add. Only the CAS-lock owner mutates
// Storage; boxes are never removed through shared access. Reading, resetting
// or freeing packet storage requires &mut self, excluding every producer.
unsafe impl Sync for CommandBuffer {}

struct Lock<'a>(&'a AtomicI32);
impl Drop for Lock<'_> {
    fn drop(&mut self) {
        self.0.swap(0, Ordering::SeqCst);
    }
}

impl CommandBuffer {
    /// Actual queue configuration must come from its enclosing world owner.
    /// Rust allocation replaces engine allocator calls and their memory tags.
    pub fn new(chunk_size: i32, memory_tag: u32) -> Result<Self, String> {
        if chunk_size < 1 {
            return Err("command chunk size must be positive".into());
        }
        Ok(Self {
            chunk_size,
            memory_tag,
            lock: AtomicI32::new(0),
            current: AtomicPtr::new(std::ptr::null_mut()),
            storage: UnsafeCell::new(Storage {
                order: PointerList::owned_with_flags(5),
                chunks: Vec::new(),
            }),
        })
    }

    pub fn chunk_size(&self) -> i32 {
        self.chunk_size
    }
    pub fn memory_tag(&self) -> u32 {
        self.memory_tag
    }

    fn acquire(&self) -> Lock<'_> {
        while self
            .lock
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            std::hint::spin_loop();
        }
        Lock(&self.lock)
    }

    /// The failed fast reservation is deliberately retained: fa4f0 adds again
    /// after acquiring the lock, before reusing or allocating another block.
    fn reserve(&self, size: i32, grow_one_at_a_time: bool) -> Result<(*mut Chunk, usize), String> {
        let current = self.current.load(Ordering::SeqCst);
        if !current.is_null() {
            // SAFETY: current points into the retained shared-header owner.
            let chunk = unsafe { &*current };
            let end = chunk
                .reserved
                .fetch_add(size, Ordering::SeqCst)
                .wrapping_add(size);
            if end <= self.chunk_size {
                chunk.records.fetch_add(1, Ordering::SeqCst);
                return Ok((current, end.wrapping_sub(size) as usize));
            }
        }
        let guard = self.acquire();
        let current = self.current.load(Ordering::SeqCst);
        if !current.is_null() {
            // SAFETY: as above; the source reloads current under its lock.
            let chunk = unsafe { &*current };
            let end = chunk
                .reserved
                .fetch_add(size, Ordering::SeqCst)
                .wrapping_add(size);
            if end <= self.chunk_size {
                drop(guard);
                chunk.records.fetch_add(1, Ordering::SeqCst);
                return Ok((current, end.wrapping_sub(size) as usize));
            }
        }
        // SAFETY: only this CAS-lock owner may mutate the list and its handles.
        let storage = unsafe { &mut *self.storage.get() };
        let mut free = None;
        for &index in storage.order.values() {
            let index = index as usize;
            if storage.chunks[index].reserved.load(Ordering::SeqCst) == 0 {
                free = Some(index);
                break;
            }
        }
        let index = if let Some(index) = free {
            index
        } else {
            let mut data = Vec::new();
            data.try_reserve_exact(self.chunk_size as usize)
                .map_err(|e| format!("command block allocation failed: {e}"))?;
            data.resize_with(self.chunk_size as usize, || UnsafeCell::new(0));
            let index = storage.chunks.len();
            storage.chunks.push(Arc::new(Chunk {
                index,
                data: data.into_boxed_slice(),
                reserved: AtomicI32::new(0),
                records: AtomicI32::new(0),
            }));
            if storage.order.append(index as u64, grow_one_at_a_time)? == u32::MAX {
                storage.chunks.pop();
                return Err("command block pointer-list append was rejected".into());
            }
            index
        };
        let chunk = &*storage.chunks[index];
        chunk.reserved.store(size, Ordering::SeqCst);
        let current = std::ptr::from_ref(chunk).cast_mut();
        self.current.swap(current, Ordering::SeqCst);
        drop(guard);
        chunk.records.fetch_add(1, Ordering::SeqCst);
        Ok((current, 0))
    }

    /// fa6a0 copies exactly the reserved bytes. This shared entry supports the
    /// source's parallel producers; dispatch/reset require joined producers.
    pub fn append_bytes(
        &self,
        bytes: &[u8],
        grow_one_at_a_time: bool,
    ) -> Result<CommandLocation, String> {
        if bytes.len() > self.chunk_size as usize {
            return Err("command exceeds its configured block size".into());
        }
        let (pointer, offset) = self.reserve(bytes.len() as i32, grow_one_at_a_time)?;
        // SAFETY: the header is retained; reservation assigns a disjoint range.
        let chunk = unsafe { &*pointer };
        if offset > chunk.data.len() || bytes.len() > chunk.data.len() - offset {
            return Err("command reservation overflowed its block".into());
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                chunk.data.as_ptr().cast::<u8>().cast_mut().add(offset),
                bytes.len(),
            );
        }
        Ok(CommandLocation {
            chunk: chunk.index,
            offset,
        })
    }

    /// 089ea0 reads the packet's u16 byte count before calling fa6a0. Additional
    /// caller storage is not copied. Malformed host packets return an error.
    pub fn append_packet(
        &self,
        packet: &[u8],
        grow_one_at_a_time: bool,
    ) -> Result<CommandLocation, String> {
        let size = packet.get(..2).ok_or("command packet has no size header")?;
        let size = u16::from_le_bytes(size.try_into().unwrap()) as usize;
        self.append_bytes(
            packet
                .get(..size)
                .ok_or("command packet size exceeds input storage")?,
            grow_one_at_a_time,
        )
    }

    /// fa6d0 retains blocks by default and selects the first for reuse. Release
    /// drops both data/header allocations and the owned pointer-list capacity.
    pub fn reset(&mut self, release: bool) -> Result<(), String> {
        let storage = self.storage.get_mut();
        if release {
            storage.chunks.clear();
            storage.order.set_capacity(0, true)?;
            self.current.swap(std::ptr::null_mut(), Ordering::SeqCst);
        } else if storage.order.count() != 0 {
            for chunk in &storage.chunks {
                chunk.reserved.store(0, Ordering::SeqCst);
                chunk.records.store(0, Ordering::SeqCst);
            }
            let first = storage.order.values()[0] as usize;
            self.current.swap(
                std::ptr::from_ref(&*storage.chunks[first]).cast_mut(),
                Ordering::SeqCst,
            );
        }
        Ok(())
    }

    /// 089eb0 stops at the first empty block. Each block's record count is a
    /// snapshot; the byte step is reloaded AFTER dispatch, allowing a command
    /// handler to change its header. The handler receives remaining storage.
    pub fn dispatch(
        &mut self,
        mut invoke: impl FnMut(u8, &mut [u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        let storage = self.storage.get_mut();
        for &index in storage.order.values() {
            let chunk = &storage.chunks[index as usize];
            let count = chunk.records.load(Ordering::SeqCst);
            if count < 1 {
                break;
            }
            let mut offset = 0usize;
            for _ in 0..count {
                // SAFETY: &mut self excludes all producers; UnsafeCell<u8> has
                // byte layout and this slice stays within the retained block.
                let bytes = unsafe {
                    std::slice::from_raw_parts_mut(
                        chunk.data.as_ptr().cast::<u8>().cast_mut(),
                        chunk.data.len(),
                    )
                };
                let remaining = bytes
                    .get_mut(offset..)
                    .ok_or("command dispatch passed the block end")?;
                if remaining.len() < 4 {
                    return Err("command dispatch has no group header".into());
                }
                // ae1f40 selects the dispatcher by byte3, not the command's
                // operation at+4 or its byte count at+0.
                invoke(remaining[3], remaining)?;
                offset = offset
                    .checked_add(u16::from_le_bytes(remaining[..2].try_into().unwrap()) as usize)
                    .ok_or("command dispatch offset overflow")?;
            }
        }
        self.reset(false)
    }
}

#[cfg(test)]
mod tests;
