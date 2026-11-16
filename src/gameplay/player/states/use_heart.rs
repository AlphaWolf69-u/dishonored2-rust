//! Persistent UseHeart67, source8557d0/855d70/855e90/8559f0.
use super::*;
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Declaration {
    pub beat: Option<String>,
    pub lock_rune: Option<String>,
    pub unlock_rune: Option<String>,
    pub lock_charm: Option<String>,
    pub unlock_charm: Option<String>,
}
impl Declaration {
    pub fn read(fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        let reference = |name| {
            crate::entities::scene_motion::atom(fields, name)
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned)
        };
        Ok(Self {
            beat: reference("m_beat"),
            lock_rune: reference("m_lockRuneStatusInteraction"),
            unlock_rune: reference("m_unlockRuneStatusInteraction"),
            lock_charm: reference("m_lockBoneCharmStatusInteraction"),
            unlock_charm: reference("m_unlockBoneCharmStatusInteraction"),
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Runtime {
    handle: u32,
    pending: bool,
    rate: f32,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            handle: 0,
            pending: false,
            rate: 1.,
        }
    }
}
impl crate::gameplay::player::Player {
    ///855d70 changes the actual existing Midnight instance in the normal
    ///message callback. Deferring this to State.Update adds a frame of latency
    ///because the next Midnight advance precedes that state update.
    pub fn heart_playrate(&mut self, rate: f32) -> Result<(), String> {
        let index = self
            .machine
            .states
            .iter()
            .position(|state| matches!(state.decl.kind, Kind::UseHeart(_)))
            .ok_or("Player has no authored UseHeart state")?;
        self.machine.states[index].heart.rate = rate;
        if !self.machine.is_active(StateId::SLASH) {
            let handle = self.machine.states[index].heart.handle;
            if !self.body.midnight.finished(handle) {
                self.machine.states[index].heart.pending = true;
            } else {
                let Kind::UseHeart(declaration) = &self.machine.states[index].decl.kind else {
                    unreachable!()
                };
                if let Some(path) = &declaration.beat {
                    let scene =
                        self.scenes.get(path).cloned().ok_or_else(|| {
                            format!("UseHeart beat scene was not prepared: {path}")
                        })?;
                    let handle = self.start_animation_scene(&scene, 1., 10)?;
                    self.machine.states[index].heart.handle = handle;
                }
            }
            self.body
                .midnight
                .set_rate(self.machine.states[index].heart.handle, rate)?;
        }
        Ok(())
    }
}
impl StateMachine {
    pub(super) fn activate_heart(&mut self, index: usize) {
        //8557d0 clears marker flags and restores rate1. It does not fabricate
        //a beat before the Heart component emits normal11d.
        self.states[index].heart.pending = false;
        self.states[index].heart.rate = 1.;
    }
    pub(super) fn deactivate_heart(&mut self, index: usize, frame: &mut Frame) {
        let runtime = &mut self.states[index].heart;
        if runtime.handle != 0 {
            frame.midnight.stop(runtime.handle);
            runtime.handle = 0;
        }
    }
    pub(super) fn update_heart(
        &mut self,
        index: usize,
        declaration: &Declaration,
        frame: &mut Frame,
    ) {
        let slash = self.is_active(StateId::SLASH);
        let runtime = &self.states[index].heart;
        let finished = frame.midnight.finished(runtime.handle);
        if !slash && finished && runtime.pending && declaration.beat.is_some() {
            let handle = Self::play(declaration.beat.as_ref(), 1., 10, frame);
            let runtime = &mut self.states[index].heart;
            runtime.handle = handle;
            let _ = frame.midnight.set_rate(handle, runtime.rate);
            runtime.pending = false;
        } else if slash && !finished {
            frame.midnight.stop(self.states[index].heart.handle);
            self.states[index].heart.handle = 0;
        }
    }
}
