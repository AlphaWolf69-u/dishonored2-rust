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
    /// Large actor centers may lie behind their collision surface. Approach
    /// the actual nearest witness using the existing authored melee goal;
    /// never require walking inside an actor or substitute its bounds for a hit.
    pub(super) fn reached(world: &World, intended: EntityId) -> Result<bool, String> {
        let p = player(world)?;
        let (eye, axis) = p.published_camera_frame();
        let scene = world.resource::<super::super::physics_scene::PhysicsScene>();
        let live = world.resource::<crate::gameplay::entity_lifetime::LiveEntities>();
        let owner = scene.point_queries()?;
        let queries = owner.read().map_err(|_| "named contact query owner poisoned")?;
        let mut ray = crate::gameplay::world_ray_query::WorldRayQuery::default();
        let mut hits = Vec::new();
        // Same canonical ray/filter as ContactAim. Its witness merely admits
        // the debug approach; the later actual sweep still decides damage.
        queries.ray_hits(&mut ray, &crate::gameplay::world_ray_query::WorldRayRequest {
            origin: eye.to_array(), direction: (axis.x_axis * 100.).to_array(), maximum_fraction: 1.,
            hit: crate::gameplay::collision_hit::ShapeHitInfo {
                body_id: None, material_id: u16::MAX, shape_key: u32::MAX, filter_info: 0, user_data: 0,
            },
            filter: Some(crate::gameplay::collision_filter::CollisionQueryFilter {
                layers: 0x1ffff, required_flags: 0x800,
            }),
        }, 0, &mut hits)?;
        let Some(hit) = hits.iter().min_by(|a, b| a.fraction.total_cmp(&b.fraction)) else { return Ok(false) };
        if scene.collision_entity(live, &hit.target)? != Some(intended) { return Ok(false) }
        let point = Vec3::new(hit.position[0], hit.position[1], hit.position[2]);
        let reached = Observation::melee_fixture_reached(world, point)?;
        if reached {
            eprintln!("ACTION_SMOKE_NAMED_APPROACH intended={intended:?} actual_ray_point={point:?} actual_hit={:?} unchanged_authored_melee_goal=true", hit.target);
        }
        Ok(reached)
    }

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
