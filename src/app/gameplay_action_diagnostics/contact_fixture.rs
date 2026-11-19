//! Opt-in named actor fixture on the existing real-input contact observer.
//! A supplied human F8 restores only its recorded feet/yaw, once; no actor,
//! geometry, damage, animation, FX ticket, clock or hit is synthesized.
use super::*;

pub(super) struct Named {
    target: String,
    restore: Option<(Vec3, f32)>,
    previous: Option<Instant>,
    frames: u64,
    worst: Duration,
    worst_stage: Stage,
    reported: bool,
}
impl Named {
    pub(super) fn begin(world: &World, allowed: bool) -> Result<Option<Self>, String> {
        let target = std::env::var("DH2_DEBUG_ACTION_TARGET").ok();
        let capture = std::env::var_os("DH2_DEBUG_ACTION_CAPTURE");
        let Some(target) = target else {
            if capture.is_some() {
                return Err("action capture restore requires actual named contact target".into());
            }
            return Ok(None);
        };
        if !allowed || target.trim().is_empty() {
            return Err("named contact target requires melee or poke scenario and a nonempty actual managed name".into());
        }
        let restore = if let Some(path) = capture {
            let report = std::fs::read_to_string(&path)
                .map_err(|error| format!("named action F8 capture: {error}"))?;
            let level = report
                .lines()
                .find_map(|line| line.strip_prefix("Level: "))
                .ok_or("named action capture has no level")?;
            let current = world
                .resource::<super::super::state::SceneData>()
                .source_path
                .to_string_lossy()
                .replace('\\', "/");
            if !current.ends_with(&level.replace('\\', "/")) {
                return Err("named action capture belongs to a different actual level".into());
            }
            let pose = super::super::view_geometry::f8_camera_pose(&report)
                .ok_or("named action capture has invalid camera vectors")?;
            let text = report
                .lines()
                .find_map(|line| line.strip_prefix("Feet probe game_xyz: "))
                .ok_or("named action capture has no recorded feet")?;
            let values = text
                .strip_prefix('[')
                .and_then(|text| text.strip_suffix(']'))
                .ok_or("named action captured feet brackets malformed")?
                .split(',')
                .map(|value| value.trim().parse::<f32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "named action captured feet malformed")?;
            let feet: [f32; 3] = values
                .try_into()
                .map_err(|_| "named action captured feet must have three lanes")?;
            if !feet.iter().all(|lane| lane.is_finite()) {
                return Err("named action captured feet are nonfinite".into());
            }
            let forward = super::super::view_to_game(pose.forward().as_vec3());
            Some((Vec3::from(feet), forward[1].atan2(forward[0])))
        } else {
            None
        };
        eprintln!("ACTION_SMOKE_NAMED_FIXTURE target={target} captured_setup={} subsequent_controls=physical_input source_health_breakable_and_FX_required=true",restore.is_some());
        Ok(Some(Self {
            target,
            restore,
            previous: None,
            frames: 0,
            worst: Duration::ZERO,
            worst_stage: Stage::Begin,
            reported: false,
        }))
    }
    pub(super) fn target(&self) -> &str {
        &self.target
    }
    pub(super) fn restore(&mut self, world: &mut World) -> Result<(), String> {
        if let Some((feet, yaw)) = self.restore.take() {
            world
                .resource_mut::<PlayerState>()
                .player
                .as_mut()
                .ok_or("named fixture actual Player absent")?
                .teleport(feet, yaw);
            eprintln!("ACTION_SMOKE_NAMED_SETUP target={} actual_captured_feet={feet:?} actual_captured_yaw={yaw} one_restore=true",self.target);
        }
        Ok(())
    }
    pub(super) fn frame(&mut self, stage: Stage) {
        if self.reported {
            return;
        }
        let now = Instant::now();
        if let Some(previous) = self.previous.replace(now) {
            self.frames += 1;
            let gap = now.duration_since(previous);
            if gap > self.worst {
                self.worst = gap;
                self.worst_stage = stage;
            }
        }
    }
    pub(super) fn report(&mut self) {
        if self.reported {
            return;
        }
        self.reported = true;
        eprintln!("ACTION_SMOKE_NAMED_FRAME_GAP target={} intervals={} maximum_ms={} ending_stage={:?} wall_interval_only_not_function_CPU=true",
            self.target,self.frames,self.worst.as_secs_f64()*1000.,self.worst_stage);
    }
}
impl Drop for Named {
    fn drop(&mut self) {
        self.report();
    }
}
