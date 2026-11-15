//! Entity-scoped component destruction (7eac80/7eb060/7eada0).
use super::{ComponentRegistry, entity::ComponentHandle, lifecycle::{ComponentReleaseWorld, ReleaseStatus, sort_batch}};

#[derive(Clone, Copy)]
enum Phase { ExtractStop, RequestStop, StopBatch, StopCallbacks, StopFinish, StopPump,
    RestoreStop, ExtractRelease, RequestRelease, ReleaseBatch, Deinitialize, ReleaseFinish,
    ReleasePump, RestoreRelease, Destroy }

pub(crate) struct ScopedRelease {
    entity: usize,
    targets: Vec<(ComponentHandle, u32)>,
    phase: Phase,
    saved: Vec<u64>,
    collected: Vec<(ComponentHandle, u32)>,
    cursor: usize,
    count: u32,
    waiting_stop: Option<(ComponentHandle, u32)>,
}

impl ComponentRegistry {
    /// Only the target entity's original queues are isolated. Work produced
    /// by its callbacks and event pumps follows the same shared registry.
    pub fn release_entity_scoped(&mut self, entity: usize,
        targets: &[(ComponentHandle, u32)], world: &mut impl ComponentReleaseWorld) -> Result<(), String> {
        match self.poll_release_entity_scoped(entity, targets, world)? {
            ReleaseStatus::Complete => Ok(()),
            ReleaseStatus::Pending => Err("scoped component release awaits asynchronous Stop completion".into()),
        }
    }

    pub fn poll_release_entity_scoped(&mut self, entity: usize,
        targets: &[(ComponentHandle, u32)], world: &mut impl ComponentReleaseWorld) -> Result<ReleaseStatus, String> {
        if self.scoped_release_running { return Err("entity-scoped release is already active".into()); }
        if self.scoped_release.is_none() {
            // Both source helpers skip queue isolation for an empty entity.
            if targets.is_empty() { return Ok(ReleaseStatus::Complete); }
            if !self.release_idle() || !self.startup_idle() || self.release_events_pending {
                return Err("entity-scoped release requires idle component batches".into());
            }
            for &(handle, generation) in targets {
                let owner = world.owner(handle)?;
                if owner.entity != Some(entity) || owner.generation != generation {
                    return Err("entity-scoped release component identity mismatch".into());
                }
            }
            let mut retained = Vec::new();
            retained.try_reserve_exact(targets.len()).map_err(|e| format!("scoped component targets: {e}"))?;
            retained.extend_from_slice(targets);
            self.scoped_release = Some(ScopedRelease { entity, targets: retained, phase: Phase::ExtractStop,
                saved: Vec::new(), collected: Vec::new(), cursor: 0, count: 0, waiting_stop: None });
        }
        let mut progress = self.scoped_release.take().ok_or("scoped release state absent")?;
        if progress.entity != entity { self.scoped_release = Some(progress); return Err("another entity-scoped release is active".into()); }
        self.scoped_release_running = true;
        let result = self.release_entity_scoped_inner(&mut progress, world);
        self.scoped_release_running = false;
        if !matches!(result, Ok(ReleaseStatus::Complete)) { self.scoped_release = Some(progress); }
        result
    }

    fn release_entity_scoped_inner(&mut self, p: &mut ScopedRelease,
        world: &mut impl ComponentReleaseWorld) -> Result<ReleaseStatus, String> {
        loop {
            match p.phase {
                Phase::ExtractStop | Phase::ExtractRelease => {
                    let stopping = matches!(p.phase, Phase::ExtractStop);
                    let list = if stopping { &mut self.pending_stop } else { &mut self.pending_release };
                    p.saved.try_reserve(list.count() as usize).map_err(|e| format!("scoped queue isolation: {e}"))?;
                    for index in (0..list.count() as usize).rev() {
                        let value = list.values()[index];
                        if world.owner(ComponentHandle::from_packed(value)?)?.entity != Some(p.entity) {
                            list.remove_at(index as i32)?;
                            p.saved.push(value);
                        }
                    }
                    p.cursor = if stopping { 0 } else { p.targets.len() };
                    p.phase = if stopping { Phase::RequestStop } else { Phase::RequestRelease };
                }
                Phase::RequestStop => {
                    if p.cursor == p.targets.len() { p.phase = Phase::StopBatch; continue; }
                    let (handle, generation) = p.targets[p.cursor];
                    if world.owner(handle)?.generation != generation { return Err("stale scoped stop target".into()); }
                    self.request_stop(handle, world)?;
                    p.cursor += 1;
                }
                Phase::StopBatch => {
                    if self.pending_stop.count() == 0 { p.cursor = 0; p.phase = Phase::RestoreStop; continue; }
                    p.count = self.pending_stop.count();
                    self.processed_stop_count = p.count as i32;
                    sort_batch(&mut self.pending_stop, world)?;
                    p.cursor = p.count as usize;
                    p.phase = Phase::StopCallbacks;
                }
                Phase::StopCallbacks => {
                    if p.cursor == 0 { p.phase = Phase::StopFinish; continue; }
                    let handle = ComponentHandle::from_packed(self.pending_stop.values()[p.cursor - 1])?;
                    let generation = world.owner(handle)?.generation;
                    let first = match p.waiting_stop {
                        None => true,
                        Some(identity) if identity == (handle, generation) => false,
                        Some(_) => return Err("pending scoped Stop component identity changed".into()),
                    };
                    p.waiting_stop = Some((handle, generation));
                    match world.poll_stop(handle, self, first)? {
                        ReleaseStatus::Pending => return Ok(ReleaseStatus::Pending),
                        ReleaseStatus::Complete => p.waiting_stop = None,
                    }
                    p.cursor -= 1;
                }
                Phase::StopFinish => {
                    if self.pending_stop.finish_prefix(p.count, 64)? { self.queue_reset_count = self.queue_reset_count.wrapping_add(1); }
                    self.processed_stop_count = 0;
                    p.phase = Phase::StopPump;
                }
                Phase::StopPump => { world.pump_events(self)?; p.phase = Phase::StopBatch; }
                Phase::RestoreStop | Phase::RestoreRelease => {
                    let stopping = matches!(p.phase, Phase::RestoreStop);
                    let list = if stopping { &mut self.pending_stop } else { &mut self.pending_release };
                    while p.cursor < p.saved.len() {
                        let count = list.count();
                        if list.append_preserving(p.saved[p.cursor], false)? != count { return Err("scoped queue restoration failed".into()); }
                        p.cursor += 1;
                    }
                    p.saved.clear(); p.cursor = 0;
                    p.phase = if stopping { Phase::ExtractRelease } else { Phase::Destroy };
                }
                Phase::RequestRelease => {
                    if p.cursor == 0 { p.phase = Phase::ReleaseBatch; continue; }
                    let (handle, generation) = p.targets[p.cursor - 1];
                    if world.owner(handle)?.generation != generation { return Err("stale scoped release target".into()); }
                    self.request_release(handle, world)?;
                    p.cursor -= 1;
                }
                Phase::ReleaseBatch => {
                    if self.pending_release.count() == 0 { p.cursor = 0; p.phase = Phase::RestoreRelease; continue; }
                    p.count = self.pending_release.count();
                    p.collected.try_reserve(p.count as usize).map_err(|e| format!("scoped release collection: {e}"))?;
                    sort_batch(&mut self.pending_release, world)?;
                    p.cursor = p.count as usize;
                    p.phase = Phase::Deinitialize;
                }
                Phase::Deinitialize => {
                    if p.cursor == 0 { p.phase = Phase::ReleaseFinish; continue; }
                    let handle = ComponentHandle::from_packed(self.pending_release.values()[p.cursor - 1])?;
                    let generation = world.owner(handle)?.generation;
                    self.ensure_no_update_references(handle)?;
                    world.deinitialize(handle, self)?;
                    p.collected.push((handle, generation));
                    p.cursor -= 1;
                }
                Phase::ReleaseFinish => {
                    if self.pending_release.finish_prefix(p.count, 64)? { self.queue_reset_count = self.queue_reset_count.wrapping_add(1); }
                    p.phase = Phase::ReleasePump;
                }
                Phase::ReleasePump => { world.pump_events(self)?; p.phase = Phase::ReleaseBatch; }
                Phase::Destroy => {
                    if p.cursor == p.collected.len() { return Ok(ReleaseStatus::Complete); }
                    let (handle, generation) = p.collected[p.cursor];
                    if world.owner(handle)?.generation != generation { return Err("stale scoped destruction target".into()); }
                    self.ensure_no_update_references(handle)?;
                    world.released(handle, self)?;
                    p.cursor += 1;
                }
            }
        }
    }
}
