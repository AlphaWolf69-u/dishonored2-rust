//!60a540/60af50/60ad40 contact-instance admission journal. Playback, family
//!pools, callbacks, clocks and RNG belong to the SAME existing FX manager.
//!The manager's exclusive borrow replaces the native40 lock/hash bucket scan.
use std::sync::Arc;
struct Bucket<D> {
    definition: Option<Arc<D>>,
    handles: [u32; 4],
    length: usize,
}
impl<D> Default for Bucket<D> {
    fn default() -> Self {
        Self {
            definition: None,
            handles: [0; 4],
            length: 0,
        }
    }
}
pub enum Admission {
    Available { oldest: Option<u32> },
    Exhausted,
}
pub struct ContactInstances<D> {
    buckets: [Bucket<D>; 16],
}
impl<D> Default for ContactInstances<D> {
    fn default() -> Self {
        Self {
            buckets: std::array::from_fn(|_| Bucket::default()),
        }
    }
}
impl<D> ContactInstances<D> {
    pub fn is_empty(&self) -> bool {
        self.buckets.iter().all(|bucket| bucket.length == 0)
    }
    ///Source60a510 sets flags440:fixed inline4, not a growable small-vector.
    ///60af50 explicitly stops oldest1a31f0 then ordered-removes when full.
    ///The caller performs that real stop before insert; a refusal is not a
    ///pending resource read and must not be retried as a different packet.
    pub fn admission(&self, definition: &Arc<D>) -> Admission {
        if let Some(bucket) = self.buckets.iter().find(|bucket| {
            bucket
                .definition
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, definition))
        }) {
            return Admission::Available {
                oldest: (bucket.length == 4).then_some(bucket.handles[0]),
            };
        }
        if self
            .buckets
            .iter()
            .any(|bucket| bucket.definition.is_none())
        {
            Admission::Available { oldest: None }
        } else {
            Admission::Exhausted
        }
    }
    pub fn insert(&mut self, definition: Arc<D>, handle: u32) -> Result<(), String> {
        let index = self
            .buckets
            .iter()
            .position(|bucket| {
                bucket
                    .definition
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &definition))
            })
            .or_else(|| {
                self.buckets
                    .iter()
                    .position(|bucket| bucket.definition.is_none())
            })
            .ok_or("contact definition bucket was not admitted")?;
        let bucket = &mut self.buckets[index];
        if bucket.length >= 4 {
            return Err("contact oldest instance was not stopped before replacement".into());
        }
        if bucket.handles[..bucket.length].contains(&handle) {
            return Err("contact instance registered twice".into());
        }
        bucket.definition = Some(definition);
        bucket.handles[bucket.length] = handle;
        bucket.length += 1;
        Ok(())
    }
    ///60ad40/60aee0 preserves remaining order and returns an empty bucket to
    ///the actual pool. Its declaration lease cannot alias a newer generation.
    pub fn remove(&mut self, handle: u32) {
        for bucket in &mut self.buckets {
            if let Some(index) = bucket.handles[..bucket.length]
                .iter()
                .position(|&current| current == handle)
            {
                bucket.handles.copy_within(index + 1..bucket.length, index);
                bucket.length -= 1;
                if bucket.length == 0 {
                    bucket.definition = None;
                }
                return;
            }
        }
    }
}
