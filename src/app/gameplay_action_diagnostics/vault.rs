//! Observe the approach retained in the streaming report for F8 1795104805498.
use super::*;

pub(super) struct Observation {
    input: [Input; 2],
    started: Instant,
    driving: bool,
    entered: bool,
    exited: Option<Vec3>,
    direction: Vec3,
    jump: Input,
    jump_pressed: bool,
    release_jump: bool,
    saw_clearance: bool,
    maximum_speed: f32,
}

impl Observation {
    pub(super) fn begin(world: &mut World) -> Result<Self, String> {
        use crate::gameplay::player::controller::{ACTION_JUMP, ACTION_MOVE_FORWARD, ACTION_SPRINT};
        // Save positions are camera positions. Preserve the live eye offset
        // when restoring feet, then let ordinary support queries settle them.
        // Use the saved forward axis, with the lateral offset already
        // applied, to enter the same side of the table as the capture.
        let approach = Vec3::new(15.239, 100.149, 14.484);
        let direction = Vec3::new(0.009, 0.998, 0.).normalize();
        let p = player(world)?;
        let feet = approach - (p.published_camera_frame().0 - p.feet());
        world.resource_mut::<PlayerState>().player.as_mut()
            .ok_or("vault observation player absent")?
            .teleport(feet, direction.y.atan2(direction.x));
        let capture = input_for(19)?;
        send(world, capture, true)?;
        send(world, capture, false)?;
        let input = [input_for(ACTION_MOVE_FORWARD)?, input_for(ACTION_SPRINT)?];
        Ok(Self { input, started: Instant::now(), driving: false, entered: false,
            exited: None, direction, jump: input_for(ACTION_JUMP)?, jump_pressed: false,
            release_jump: false, saw_clearance: false, maximum_speed: 0. })
    }

    pub(super) fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        let p = player(world)?;
        let feet = p.feet();
        let vault = p.machine.is_active(StateId::VAULT);
        let speed = p.velocity().truncate().length();
        self.maximum_speed = self.maximum_speed.max(speed);
        self.saw_clearance |= p.vault_sensor.can_vault;
        if self.started.elapsed() > Duration::from_secs(35) {
            let mut report = String::new();
            p.write_combat_diagnostics(&mut report);
            eprintln!("VAULT_CAPTURE_REPLAY_END entered={} exited={:?} feet={feet:?}{report}",
                self.entered, self.exited);
            for input in self.input { send(world, input, false)?; }
            if self.release_jump { send(world, self.jump, false)?; }
            return Err(format!("F8 vault approach timed out entered={} exited={:?} feet={feet:?} jump_pressed={} saw_clearance={} maximum_speed={}",
                self.entered, self.exited, self.jump_pressed, self.saw_clearance, self.maximum_speed));
        }
        if !self.driving {
            for input in self.input { send(world, input, true)?; }
            self.driving = true;
            return Ok(false);
        }
        if self.release_jump {
            send(world, self.jump, false)?;
            self.release_jump = false;
        } else if !self.jump_pressed && p.vault_sensor.can_vault {
            let state = p.machine.state(StateId::VAULT).ok_or("vault declaration absent")?;
            let crate::gameplay::player::states::Kind::Vault(decl) = &state.decl.kind else {
                return Err("vault replay found a different declared state".into());
            };
            if speed >= decl.min_speed {
                send(world, self.jump, true)?;
                self.jump_pressed = true;
                self.release_jump = true;
            }
        }
        if vault && !self.entered {
            self.entered = true;
            eprintln!("VAULT_CAPTURE_REPLAY_ENTER feet={feet:?}");
        } else if !vault && self.entered {
            let exit = *self.exited.get_or_insert(feet);
            if (feet - exit).dot(self.direction) > player(world)?.physics.active_shape().radius {
                eprintln!("ACTION_SMOKE_PASS check=walk_clear_after_captured_vault_approach exit={exit:?} feet={feet:?}");
                for input in self.input { send(world, input, false)?; }
                return Ok(true);
            }
        }
        Ok(false)
    }
}
