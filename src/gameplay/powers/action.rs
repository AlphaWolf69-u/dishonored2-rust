//! arkPlayerStateUsePower scene lifecycle, 140856160/4e0/c50.
//! All handles refer to real prepared Midnight scenes from the arm owner.
use super::{Cast, Phase};

pub trait Scenes {
    type Error;
    fn start(&mut self, declaration: &str) -> Result<u32, Self::Error>;
    fn finished(&self, handle: u32) -> bool;
    fn stop(&mut self, handle: u32);
    fn cancel_sound(&mut self, declaration: &str);
}
#[derive(Default)]
pub struct Action { entry: Option<u32>, loop_scene: Option<u32> }
impl Action {
    pub fn entry_handle(&self) -> u32 { self.entry.unwrap_or(0) }
    pub fn scene_handle(&self) -> u32 { self.loop_scene.or(self.entry).unwrap_or(0) }
    /// 140856160 chooses the sliding variant only when it is present.
    /// Call only after inventory, player-state and family admission succeed.
    pub fn begin<S: Scenes>(&mut self, cast: &mut Cast, sliding: bool, scenes: &mut S) -> Result<bool, S::Error> {
        if cast.phase() != Phase::Idle { return Ok(false); }
        let settings = &cast.settings;
        let declaration = variant(sliding, &settings.in_midnight, &settings.in_midnight_slide);
        // Resolve and start first so a failed acquisition cannot leave a
        // camera modifier or an active cast with no matching owner.
        let entry = declaration.map(|path| scenes.start(path)).transpose()?;
        cast.begin(); self.entry = entry; self.loop_scene = None;
        Ok(true)
    }
    /// 140856c50 waits for both handles to finish, then starts the authored
    /// loop again while the power has not cast. A loop Midnight can itself
    /// be finite; its actual completion drives re-entry, rather than a timer.
    pub fn update<S: Scenes>(&mut self, cast: &Cast, sliding: bool, scenes: &mut S) -> Result<(), S::Error> {
        if cast.phase() != Phase::Aiming { return Ok(()); }
        if self.loop_scene.is_some_and(|handle| !scenes.finished(handle)) { return Ok(()); }
        if self.entry.is_some_and(|handle| !scenes.finished(handle)) { return Ok(()); }
        if let Some(path) = variant(sliding, &cast.settings.loop_midnight, &cast.settings.loop_midnight_slide) {
            self.loop_scene = Some(scenes.start(path)?);
        }
        Ok(())
    }
    /// 1408564e0: normal out animation only follows a committed cast;
    /// otherwise stop both scenes and play the actual cancellation scene.
    /// Returns whether the camera attribute modifier must be removed.
    pub fn finish<S: Scenes>(&mut self, cast: &mut Cast, sliding: bool, scenes: &mut S) -> Result<bool, S::Error> {
        if cast.phase() == Phase::Idle { return Ok(false); }
        let committed = cast.phase() == Phase::Cast;
        let settings = &cast.settings;
        let out = variant(sliding, &settings.out_midnight, &settings.out_midnight_slide);
        if committed && out.is_some() {
            if let Some(handle) = self.loop_scene.take() { scenes.stop(handle); }
            if let Some(path) = out { scenes.start(path)?; }
        } else {
            self.stop(scenes);
            if let Some(path) = &settings.cancel_midnight { scenes.start(path)?; }
            if let Some(path) = &settings.cancel_sound { scenes.cancel_sound(path); }
        }
        self.entry = None; self.loop_scene = None;
        Ok(cast.finish())
    }
    /// Entity retirement cancels live instances before prepared leases drop.
    pub fn stop<S: Scenes>(&mut self, scenes: &mut S) {
        if let Some(handle) = self.entry.take() { scenes.stop(handle); }
        if let Some(handle) = self.loop_scene.take() { scenes.stop(handle); }
    }
}
fn variant<'a>(sliding: bool, normal: &'a Option<String>, slide: &'a Option<String>) -> Option<&'a str> {
    if sliding { slide.as_deref().or(normal.as_deref()) } else { normal.as_deref() }
}
