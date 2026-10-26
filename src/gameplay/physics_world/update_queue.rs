//! Bounded transform-update tasks, including both world enqueue sites
//! (8a60e0/8a6030). Stable handles replace queued process addresses.
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Mutex,
};

pub const UPDATE_CAPACITY: usize = 0x2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjectReference<T> {
    pub generation: u32,
    pub object: Option<T>,
}
impl<T> ObjectReference<T> {
    /// 3da8f0; the generation comes from the referenced object's +18 field.
    pub fn capture(value: Option<(u32, T)>) -> Self {
        match value {
            Some((generation, object)) => Self {
                generation,
                object: Some(object),
            },
            None => Self {
                generation: u32::MAX,
                object: None,
            },
        }
    }
}
impl<T: Copy> ObjectReference<T> {
    /// 3da910 with policy0: 7e7390's mask pair is (0,0), so generation is
    /// the only admission test. A stale or absent handle cannot be dispatched.
    pub fn resolve(&self, mut generation: impl FnMut(T) -> Option<u32>) -> Option<T> {
        if self.generation == u32::MAX {
            return None;
        }
        let object = self.object?;
        (generation(object) == Some(self.generation)).then_some(object)
    }
}

pub struct UpdateIndexPool {
    head: AtomicU32,
    next: Box<[AtomicU32]>,
}
impl UpdateIndexPool {
    pub fn from_parts(head: u32, next: Vec<u32>) -> Result<Self, String> {
        if next.len() != UPDATE_CAPACITY
            || head & 0x3fff > UPDATE_CAPACITY as u32
            || next.iter().any(|n| n & 0x3fff > UPDATE_CAPACITY as u32)
        {
            return Err("update pool contains an invalid free index".into());
        }
        Ok(Self {
            head: AtomicU32::new(head),
            next: next.into_iter().map(AtomicU32::new).collect(),
        })
    }
    /// 8aefe0 increments the version by4000 only on a successful reservation.
    pub fn reserve(&self) -> Option<u32> {
        loop {
            let head = self.head.load(Ordering::SeqCst);
            let index = head & 0x3fff;
            if index == UPDATE_CAPACITY as u32 {
                return None;
            }
            let next = (head & 0xffffc000).wrapping_add(0x4000)
                | self.next[index as usize].load(Ordering::SeqCst);
            if self
                .head
                .compare_exchange(head, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Some(index);
            }
        }
    }
    pub fn head(&self) -> u32 {
        self.head.load(Ordering::SeqCst)
    }
    pub fn free_links(&self) -> impl Iterator<Item = u32> + '_ {
        self.next.iter().map(|next| next.load(Ordering::SeqCst))
    }
    /// 750ea0 pushes the released record back without incrementing the version.
    pub fn release(&self, index: u32) -> Result<(), String> {
        let next = self
            .next
            .get(index as usize)
            .ok_or("released update index is missing")?;
        loop {
            let head = self.head.load(Ordering::SeqCst);
            next.store(head & 0x3fff, Ordering::SeqCst);
            if self
                .head
                .compare_exchange(
                    head,
                    head & 0xffffc000 | index,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
    }
}

pub struct UpdateIndexQueue {
    read: AtomicU32,
    reserved: AtomicU32,
    committed: AtomicU32,
    slots: Box<[AtomicU32]>,
}

fn backoff(count: &mut u32) {
    if *count < 0x101 {
        for _ in 0..*count {
            std::hint::spin_loop();
        }
        *count *= 2;
    } else {
        std::thread::yield_now();
    }
}

impl UpdateIndexQueue {
    pub fn from_parts(
        read: u32,
        reserved: u32,
        committed: u32,
        slots: Vec<u32>,
    ) -> Result<Self, String> {
        if slots.len() != UPDATE_CAPACITY {
            return Err("update ring storage must hold8192 indices".into());
        }
        Ok(Self {
            read: AtomicU32::new(read),
            reserved: AtomicU32::new(reserved),
            committed: AtomicU32::new(committed),
            slots: slots.into_iter().map(AtomicU32::new).collect(),
        })
    }
    /// 7538c0 reserves a ring position, writes, then commits in producer order.
    pub fn push(&self, value: u32) -> bool {
        let position = loop {
            let tail = self.reserved.load(Ordering::SeqCst);
            if tail.wrapping_sub(self.read.load(Ordering::SeqCst)) as i32 > 0x1fff {
                return false;
            }
            if self
                .reserved
                .compare_exchange(
                    tail,
                    tail.wrapping_add(1),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break tail;
            }
        };
        self.slots[(position & 0x1fff) as usize].store(value, Ordering::SeqCst);
        let mut spins = 1;
        while self
            .committed
            .compare_exchange(
                position,
                position.wrapping_add(1),
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_err()
        {
            backoff(&mut spins);
        }
        true
    }
    pub fn counters(&self) -> [u32; 3] {
        [
            self.read.load(Ordering::SeqCst),
            self.reserved.load(Ordering::SeqCst),
            self.committed.load(Ordering::SeqCst),
        ]
    }
    pub fn slot(&self, position: u32) -> u32 {
        self.slots[(position & 0x1fff) as usize].load(Ordering::SeqCst)
    }
    /// Complete753850. A stored ffffffff is consumed and has the same stop
    /// result as an empty queue, as in both update-task consumers.
    pub fn pop(&self) -> Option<u32> {
        loop {
            let read = self.read.load(Ordering::SeqCst);
            if self.committed.load(Ordering::SeqCst) == read {
                return None;
            }
            let value = self.slots[(read & 0x1fff) as usize].load(Ordering::SeqCst);
            if self
                .read
                .compare_exchange(
                    read,
                    read.wrapping_add(1),
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                return (value != u32::MAX).then_some(value);
            }
        }
    }
}

pub struct ObjectUpdateQueue<T> {
    pub pool: UpdateIndexPool,
    pub queue: UpdateIndexQueue,
    entries: Box<[Mutex<ObjectReference<T>>]>,
}
impl<T: Copy> ObjectUpdateQueue<T> {
    pub fn from_parts(
        pool: UpdateIndexPool,
        queue: UpdateIndexQueue,
        entries: Vec<ObjectReference<T>>,
    ) -> Result<Self, String> {
        if entries.len() != UPDATE_CAPACITY {
            return Err("update reference storage must hold8192 entries".into());
        }
        Ok(Self {
            pool,
            queue,
            entries: entries.into_iter().map(Mutex::new).collect(),
        })
    }
    /// Complete8a60e0: a full free pool drops the request; a reserved record
    /// waits for ring space. The actor's frame stamp has already been written.
    pub fn enqueue(&self, reference: ObjectReference<T>) -> Result<bool, String> {
        let Some(index) = self.pool.reserve() else {
            return Ok(false);
        };
        *self.entries[index as usize]
            .lock()
            .map_err(|_| "update reference lock is poisoned")? = reference;
        let mut spins = 1;
        while !self.queue.push(index) {
            backoff(&mut spins);
        }
        Ok(true)
    }
    pub fn entry(&self, index: usize) -> Result<ObjectReference<T>, String> {
        self.entries
            .get(index)
            .ok_or("update reference index is missing")?
            .lock()
            .map(|value| *value)
            .map_err(|_| "update reference lock is poisoned".into())
    }
}

/// Shared meaningful state of8a3580/8a3650, independent of their task vtables.
/// The pre/post phase owns its actual actor update callback and frame markers.
pub struct ObjectUpdateTask<T> {
    pub updates: ObjectUpdateQueue<T>,
    pub frame: i32,
    pub stamp: i32,
    sentinel: u32,
}
impl<T: Copy> ObjectUpdateTask<T> {
    pub fn new() -> Self {
        let pool = UpdateIndexPool::from_parts(0, (1..=UPDATE_CAPACITY as u32).collect())
            .expect("fixed update free list is valid");
        let queue = UpdateIndexQueue::from_parts(0, 0, 0, vec![0; UPDATE_CAPACITY])
            .expect("fixed update ring capacity is valid");
        let updates = ObjectUpdateQueue::from_parts(
            pool,
            queue,
            vec![ObjectReference::capture(None); UPDATE_CAPACITY],
        )
        .expect("fixed update entry capacity is valid");
        let sentinel = updates
            .pool
            .reserve()
            .expect("new update pool has a sentinel slot");
        Self {
            updates,
            frame: 0,
            stamp: 0,
            sentinel,
        }
    }
    pub fn sentinel(&self) -> u32 {
        self.sentinel
    }
    /// 751370 inserts one permanent sentinel per scheduled task worker.
    pub fn publish_sentinels(&self, count: u32) {
        if count as i32 <= 0 {
            return;
        }
        for _ in 0..count {
            let mut spins = 1;
            while !self.updates.queue.push(self.sentinel) {
                backoff(&mut spins);
            }
        }
    }
    /// Shared drain/recycle sequence of755030/7551c0. A consumed sentinel ends
    /// this task batch without releasing its permanently reserved pool slot.
    pub fn drain(
        &self,
        mut dispatch: impl FnMut(ObjectReference<T>, i32, i32) -> Result<(), String>,
    ) -> Result<(), String> {
        while let Some(index) = self.updates.queue.pop() {
            if index == self.sentinel {
                break;
            }
            let reference = self.updates.entry(index as usize)?;
            let result = dispatch(reference, self.frame, self.stamp);
            self.updates.pool.release(index)?;
            result?;
        }
        Ok(())
    }
}
impl<T: Copy> Default for ObjectUpdateTask<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T> Drop for ObjectUpdateTask<T> {
    fn drop(&mut self) {
        // 8a4460/8a44b0 and the non-allocating portion of their deleting
        // destructors return the permanent sentinel before owned storage dies.
        self.updates
            .pool
            .release(self.sentinel)
            .expect("task owns its valid sentinel index");
    }
}

#[cfg(test)]
#[path = "update_queue_tests.rs"]
mod tests;
