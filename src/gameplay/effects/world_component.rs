//! arkCpntFX's deferred creation and target lifetime (4b2200). The reference
//! actor and effect manager remain real world producers, not renderer clocks.

#[derive(Clone, Copy)]
pub struct ReferenceReadiness {
    pub state: u8,
    pub pending_binding: bool,
}

pub trait WorldFxComponentHost {
    fn has_effect(&self) -> bool;
    fn reference_readiness(&self) -> Option<ReferenceReadiness>;
    /// 4b1d80 returns the manager's instance ID only after successful creation.
    fn play(&mut self) -> Result<Option<u32>, String>;
    fn target_present(&self) -> bool;
    fn target_alive(&mut self) -> Result<bool, String>;
    fn update_target_pose(&mut self, handle: u32) -> Result<(), String>;
    fn request_stop(&mut self, handle: u32) -> Result<(), String>;
    fn update_source_velocity(&mut self) -> Result<(), String>;
    /// 4b1ad0 transfers completion ownership and clears source/model pointers.
    /// A stale ID resolves to None; it is never treated as a live instance.
    fn detach(&mut self, handle: Option<u32>) -> Result<Option<u32>, String>;
    fn stop_immediately(&mut self, handle: u32) -> Result<(), String>;
}

#[derive(Default)]
pub struct WorldFxComponent {
    handle: Option<u32>,
    started: bool,
    stop_requested: bool,
    reference_revision: u64,
}

impl WorldFxComponent {
    ///Host publication receipt for the same source125 callback. A pending
    ///unbind dominates later rebinds because its immediate stop is irreversible.
    pub fn reference_event_once(&mut self, revision:u64,parent_present:bool,
        stop_on_parent_removal:bool,host:&mut impl WorldFxComponentHost)->Result<(),String>{
        if self.reference_revision==revision{return Ok(());}
        self.reference_event(parent_present,stop_on_parent_removal,host)?;
        self.reference_revision=revision;Ok(())
    }
    pub fn handle(&self) -> Option<u32> {
        self.handle
    }

    /// Creation is retried until the actual reference actor has started and
    /// finished its pending binding. A successful creation is never repeated.
    pub fn tick(
        &mut self,
        stop_on_target_removal: bool,
        host: &mut impl WorldFxComponentHost,
    ) -> Result<(), String> {
        if !self.started
            && host.has_effect()
            && host
                .reference_readiness()
                .is_none_or(|reference| reference.state & 0xc > 7 && !reference.pending_binding)
        {
            if let Some(handle) = host.play()? {
                self.handle = Some(handle);
                self.started = true;
                self.stop_requested = false;
            }
        }
        if let Some(handle) = self.handle {
            if host.target_present() {
                if !host.target_alive()? {
                    if stop_on_target_removal && !self.stop_requested {
                        host.request_stop(handle)?;
                        self.stop_requested = true;
                    }
                } else {
                    host.update_target_pose(handle)?;
                }
            }
            host.update_source_velocity()?;
        }
        Ok(())
    }

    /// 4b22d0 detaches on either reference event. A missing parent additionally
    /// stops the detached instance when its authored removal flag is enabled.
    pub fn reference_event(
        &mut self,
        parent_present: bool,
        stop_on_parent_removal: bool,
        host: &mut impl WorldFxComponentHost,
    ) -> Result<(), String> {
        if let Some(handle) = host.detach(self.handle)? {
            if !parent_present && stop_on_parent_removal {
                host.stop_immediately(handle)?;
            }
        }
        Ok(())
    }

    /// 4b1cc0 always detaches before immediate stop and invalidating the ID.
    pub fn teardown(&mut self, host: &mut impl WorldFxComponentHost) -> Result<(), String> {
        if let Some(handle) = host.detach(self.handle)? {
            host.stop_immediately(handle)?;
        }
        self.handle = None;
        Ok(())
    }
}
