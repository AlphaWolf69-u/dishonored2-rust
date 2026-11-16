//! PlayHeartBark66:844670/844750/844830. The use scene belongs to the
//! action; speech selection and completion belong to the actual Heart actor.
use super::*;
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub action: super::super::combat::ActionDecl,
    pub use_midnight: Option<String>,
    pub delilah_angle: f32,
    pub delilah_distance: f32,
}
impl Declaration {
    pub fn read(fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        use crate::entities::scene_motion::atom;
        Ok(Self {
            action: super::super::combat::ActionDecl::read(fields)?,
            use_midnight: atom(fields, "m_useMidnight")
                .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned),
            delilah_angle: crate::gameplay::damage::number(fields, "m_delilahAngleToTrigger", 40.)?,
            delilah_distance: crate::gameplay::damage::number(
                fields,
                "m_delilahDistanceToTrigger",
                10.,
            )?,
        })
    }
}
impl StateMachine {
    fn execute_heart_bark(&mut self, index: usize, decl: &Declaration, frame: &mut Frame) {
        if let Some(equipment) = frame.heart_entity {
            //8448f0 visits the actual AI roster for a Delilah-soul receiver.
            //That family is not admitted in the rewrite; source absence of
            //a receiver follows the ordinary6123e0 branch, never a fake soul.
            frame
                .heart_requests
                .push_back(super::super::weapons::heart::UseRequest {
                    equipment,
                    target: frame.heart_target,
                });
            Self::play(decl.use_midnight.as_ref(), 1., 10, frame);
        }
        self.quit(self.states[index].decl.id); //844750 ->854190
    }
    pub(super) fn activate_heart_bark(
        &mut self,
        index: usize,
        decl: &Declaration,
        frame: &mut Frame,
    ) {
        use super::super::combat::ActionStep;
        if self.states[index].heart_bark.activate(&decl.action, false) == ActionStep::Execute {
            let handle = Self::play(
                decl.action.midnight.as_ref(),
                1.,
                self.states[index].decl.flags.body_parts,
                frame,
            );
            self.states[index].midnight = handle;
            self.states[index]
                .heart_bark
                .after_scene(handle, &decl.action);
            //854e50 invokes C8 (start the common scene), not B8 (execute).
            //The derived callback is selected by the actual8555a0 update.
        }
    }
    pub(super) fn update_heart_bark(
        &mut self,
        index: usize,
        decl: &Declaration,
        frame: &mut Frame,
    ) {
        let action = frame
            .controller
            .mapping
            .iter()
            .find(|m| m.state == self.states[index].decl.id)
            .map_or(0, |m| m.action);
        let step = self.states[index].heart_bark.update(
            frame.dt,
            &decl.action,
            frame.controller.down(action),
            frame.midnight.finished(self.states[index].midnight),
            false,
        );
        match step {
            super::super::combat::ActionStep::Execute => {
                self.execute_heart_bark(index, decl, frame)
            }
            super::super::combat::ActionStep::Quit => self.quit(self.states[index].decl.id),
            _ => {}
        }
    }
}
impl crate::gameplay::player::Player {
    ///854fd0/855750 normalF6 dispatches only to the matching actual action
    ///scene. The use animation remains distinct from the common Action scene.
    pub fn heart_action_message(&mut self, handle: u32) {
        let Some(state) = self
            .machine
            .state(StateId::PLAY_HEART_BARK)
            .filter(|state| state.active)
        else {
            return;
        };
        let Kind::PlayHeartBark(declaration) = &state.decl.kind else {
            return;
        };
        if state.heart_bark.message(&declaration.action, handle)
            != super::super::combat::ActionStep::Execute
        {
            return;
        }
        let path = declaration.use_midnight.clone();
        if let Some(equipment) = self.heart_entity {
            self.heart_requests
                .push_back(super::super::weapons::heart::UseRequest {
                    equipment,
                    target: self.ranged_target_entity,
                });
            if let Some(path) = path {
                match self
                    .scenes
                    .get(&path)
                    .cloned()
                    .ok_or_else(|| format!("Heart use Midnight not prepared: {path}"))
                    .and_then(|scene| self.start_animation_scene(&scene, 1., 10))
                {
                    Ok(_) => {}
                    Err(error) => self.body.midnight.last_start_error = Some(error),
                }
            }
        }
        self.machine.quit(StateId::PLAY_HEART_BARK);
    }
}
