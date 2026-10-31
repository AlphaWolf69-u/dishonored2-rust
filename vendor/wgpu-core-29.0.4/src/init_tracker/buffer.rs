use super::{InitTracker, MemoryInitKind};
use crate::resource::Buffer;
use alloc::sync::Arc;
use core::ops::Range;

#[derive(Debug, Clone)]
pub(crate) struct BufferInitTrackerAction {
    pub buffer: Arc<Buffer>,
    pub range: Range<wgt::BufferAddress>,
    pub kind: MemoryInitKind,
}

pub(crate) type BufferInitTracker = InitTracker<wgt::BufferAddress>;

impl BufferInitTracker {
    /// Checks if an action has/requires any effect on the initialization status
    /// and shrinks its range if possible.
    pub(crate) fn check_action(
        &self,
        action: &BufferInitTrackerAction,
    ) -> Option<BufferInitTrackerAction> {
        self.create_action(&action.buffer, action.range.clone(), action.kind)
    }

    /// Creates an action if it would have any effect on the initialization
    /// status and shrinks the range if possible.
    pub(crate) fn create_action(
        &self,
        buffer: &Arc<Buffer>,
        query_range: Range<wgt::BufferAddress>,
        kind: MemoryInitKind,
    ) -> Option<BufferInitTrackerAction> {
        self.check(query_range)
            .map(|range| BufferInitTrackerAction {
                buffer: buffer.clone(),
                range,
                kind,
            })
    }
}

/// DH2 (project patch): `create_action` through the buffer's status lock,
/// skipped once the whole buffer is initialized: the action would be empty.
/// The shared lock acquisition writes the lock word, which every recording
/// thread otherwise contends on for each bound, vertex, index or indirect
/// buffer. DH2_WGPU_INIT_LOCKS=1 always takes the lock, for A/B measurement.
pub(crate) fn buffer_init_action(
    buffer: &Arc<Buffer>,
    query_range: Range<wgt::BufferAddress>,
    kind: MemoryInitKind,
) -> Option<BufferInitTrackerAction> {
    use core::sync::atomic::Ordering;
    if super::skip_initialized_locks() && buffer.initialized_hint.load(Ordering::Acquire) {
        return None;
    }
    let status = buffer.initialization_status.read();
    let action = status.create_action(buffer, query_range, kind);
    if status.uninitialized_ranges.is_empty() {
        buffer.initialized_hint.store(true, Ordering::Release);
    }
    action
}
