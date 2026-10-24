//! World and trigger winds sampled by camera fog, physics and vegetation:
//! 77edf0, 77fbc0, 77fec0, 780010/90/b0, and the wind manager's 7b7be0.
use crate::noise::PERMUTATION;
use bevy_math::Vec3;

pub use crate::assets::entities::wind::WindSettings;

pub struct Wind {
    settings: WindSettings,
    direction: Vec3,
    main_speed: f32,
    speed_variation: f32,
    pub previous_phase: f32,
    pub phase: f32,
}
impl Wind {
    pub fn new(settings: WindSettings) -> Self {
        let d = settings.direction;
        let squared = (d.y * d.y + d.x * d.x + d.z * d.z).max(f32::from_bits(0x00800000));
        let direction = d * (1. / squared.sqrt());
        Self {
            settings,
            direction,
            main_speed: settings.main_speed,
            speed_variation: settings.speed_variation,
            previous_phase: 0.,
            phase: 0.,
        }
    }
    pub fn set_speeds(&mut self, main: f32, variation: f32) {
        self.main_speed = main;
        self.speed_variation = variation;
    }
    pub fn reset_speeds(&mut self) {
        self.set_speeds(self.settings.main_speed, self.settings.speed_variation);
    }
    pub fn advance(&mut self, actor_delta: f32) {
        self.previous_phase = self.phase;
        self.phase = actor_delta * self.main_speed + self.phase;
    }
    pub fn velocity(&self, position: Vec3) -> Vec3 {
        let d = self.direction;
        let coordinate = (position.y * d.y + d.x * position.x + position.z * d.z) - self.phase;
        let noise = fractal_noise(
            self.settings.octaves,
            self.settings.persistence,
            self.settings.frequency,
            coordinate,
        );
        let speed = noise * self.speed_variation + self.main_speed;
        let mut seed = (speed * 1000.) as i64 as u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 10) & 0x7fff) as f32 * f32::from_bits(0x38000100) * 2. - 1.
        };
        let x = random();
        let y = random();
        let z = random();
        Vec3::new(
            (x * self.settings.noise_ratio + d.x) * speed,
            (y * self.settings.noise_ratio + d.y) * speed,
            (z * self.settings.noise_ratio + d.z) * speed,
        )
    }
}

impl Wind {
    /// 77fbc0: one vegetation windInfo record, `[previousWind, wind]` at
    /// +0/+0x10. Each half is the speed-scaled direction normalized again,
    /// with its length in w; the noise ratio's random jitter is not used.
    pub fn center_wind(&self, center: Vec3) -> [[f32; 4]; 2] {
        let d = self.direction;
        let sample = |offset: f32| {
            let coordinate = (center.x * d.x + d.y * center.y + center.z * d.z) - offset;
            let noise = fractal_noise(
                self.settings.octaves,
                self.settings.persistence,
                self.settings.frequency,
                coordinate,
            );
            let speed = noise * self.speed_variation + self.main_speed;
            let v = d * speed;
            let squared = v.y * v.y + v.x * v.x + v.z * v.z;
            let scale = 1. / squared.max(f32::from_bits(0x00800000)).sqrt();
            [v.x * scale, v.y * scale, scale * v.z, scale * squared]
        };
        [sample(self.previous_phase), sample(self.phase)]
    }
}

/// The wind component a center points at (world+0x248 in 339160).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindOwner {
    /// The registered world wind (882630, world+0x4e8): the same component
    /// that drives camera fog, not a copy.
    World,
    /// `WindManager::triggers[index]`.
    Trigger(usize),
}

/// arkWindManager (game+0x30f110) after PostLoadMap 7b7be0: the wind owning
/// each shared wind center (FUN_140295e40's list, world+0x240/+0x248).
pub struct WindManager {
    pub triggers: Vec<Wind>,
    /// Each trigger wind's 1405ff500 priority, start state and shape, for
    /// rigid bodies that track the triggers they overlap.
    pub trigger_shapes: Vec<(i32, bool, super::trigger_volume::TriggerVolume)>,
    /// Per center; None where no world wind is registered (a null pointer).
    assignments: Vec<Option<WindOwner>>,
    /// Centers whose top-priority triggers differ only by physics query order.
    pub ambiguous_centers: usize,
}

/// A trigger-linked wind and its arkCpntWind lifecycle state at PostLoadMap.
pub struct TriggerWindSource {
    pub entity: String,
    pub settings: WindSettings,
    pub priority: i32,
    pub volume: super::trigger_volume::TriggerVolume,
    /// Component state +0x1c bits 2..3 >= 2 (started, or stop pending).
    pub started: bool,
}

/// Trigger winds of a level with their installed shapes. Spawned map
/// components are started before PostLoadMap: every wind in the live
/// madinventor_p capture had state 2 (+0x1c = 0x2000000a).
pub fn trigger_wind_sources(
    catalog: &crate::archive::Catalog,
    document: &crate::entities::Document,
) -> Result<Vec<TriggerWindSource>, String> {
    let read = |name: &str| {
        catalog
            .read(name)
            .map(|bytes| bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
            .map_err(|error| error.to_string())
    };
    crate::entities::wind::trigger_winds(document, read)?
        .into_iter()
        .map(|trigger| {
            let model = trigger.model.as_deref().ok_or_else(|| {
                format!("{}: trigger wind has no renderModelInfo shape", trigger.entity)
            })?;
            Ok(TriggerWindSource {
                entity: trigger.entity.clone(),
                settings: trigger.wind.settings,
                priority: trigger.wind.priority,
                volume: super::trigger_volume::TriggerVolume::load(
                    catalog,
                    &trigger.entity,
                    model,
                    trigger.position,
                    trigger.rotation,
                )?,
                started: true,
            })
        })
        .collect()
}

impl WindManager {
    /// 7b7be0: every center starts on the world wind (882630, world+0x4e8).
    /// Each started trigger wind whose shape contains the center is inserted
    /// before the first candidate of lower-or-equal 1405ff500 priority; the
    /// head of that list replaces the default.
    pub fn post_load_map(
        world_registered: bool,
        triggers: Vec<TriggerWindSource>,
        centers: &[[f32; 3]],
    ) -> Self {
        let mut ambiguous_centers = 0;
        let assignments = centers
            .iter()
            .map(|&center| {
                let mut best: Option<usize> = None;
                let mut tied = false;
                for (index, trigger) in triggers.iter().enumerate() {
                    if !trigger.started || !trigger.volume.contains(center) {
                        continue;
                    }
                    match best {
                        Some(current) if triggers[current].priority > trigger.priority => {}
                        Some(current) if triggers[current].priority == trigger.priority => {
                            tied |= triggers[current].settings != trigger.settings;
                        }
                        _ => {
                            best = Some(index);
                            tied = false;
                        }
                    }
                }
                ambiguous_centers += usize::from(tied);
                best.map(WindOwner::Trigger)
                    .or(world_registered.then_some(WindOwner::World))
            })
            .collect();
        let (triggers, trigger_shapes) = triggers
            .into_iter()
            .map(|t| (Wind::new(t.settings), (t.priority, t.started, t.volume)))
            .unzip();
        Self {
            triggers,
            trigger_shapes,
            assignments,
            ambiguous_centers,
        }
    }

    pub fn assignment(&self, center: usize) -> Option<WindOwner> {
        self.assignments.get(center).copied().flatten()
    }

    /// 7800b0 on every trigger wind component, with its owner's actor delta.
    /// The world wind advances with its own owner.
    pub fn advance(&mut self, actor_delta: f32) {
        for wind in &mut self.triggers {
            wind.advance(actor_delta);
        }
    }

    /// 339160 -> 77fbc0 for one center; None for a null wind pointer.
    pub fn center_wind(
        &self,
        world: Option<&Wind>,
        center: usize,
        position: [f32; 3],
    ) -> Option<[[f32; 4]; 2]> {
        let wind = match self.assignment(center)? {
            WindOwner::World => world?,
            WindOwner::Trigger(index) => &self.triggers[index],
        };
        Some(wind.center_wind(Vec3::from_array(position)))
    }
}

/// 10de00: ridged accumulation modulates each next octave by the sum so far.
fn fractal_noise(octaves: i32, persistence: f32, mut frequency: f32, position: f32) -> f32 {
    let mut sum: f32 = 0.;
    let mut amplitude = 1.;
    for _ in 0..octaves {
        let sample = simplex_noise(frequency * position);
        frequency = frequency * 2.;
        let weighted = sample * amplitude;
        amplitude = amplitude * persistence;
        sum = sum + (1. - sum.abs()) * weighted;
    }
    sum
}

/// 10df10: the one-dimensional simplex kernel, including the nonpositive
/// lattice convention. The hash uses the standard Perlin permutation, also
/// published at https://mrl.cs.nyu.edu/~perlin/noise/ .
fn simplex_noise(mut position: f32) -> f32 {
    let mut lattice = position as i32;
    if position <= 0. {
        lattice = lattice.wrapping_sub(1);
    }
    position = position - lattice as f32;
    let next = position - 1.;
    let weight = 1. - position * position;
    let weight = weight * weight;
    let next_weight = 1. - next * next;
    let next_weight = next_weight * next_weight;
    let gradient = |index: i32| {
        let hash = PERMUTATION[(index as u32 & 255) as usize];
        let value = f32::from(hash & 7) + 1.;
        if hash & 8 != 0 {
            -value
        } else {
            value
        }
    };
    (gradient(lattice.wrapping_add(1)) * next * next_weight * next_weight
        + gradient(lattice) * position * weight * weight)
        * f32::from_bits(0x3eca3d71)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn wind_matches_installed_noise_and_component_samples() {
        for (case, line) in include_str!("../../tests/fixtures/wind.txt")
            .lines()
            .enumerate()
        {
            let values = line
                .split_whitespace()
                .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 19);
            let v = &values;
            let settings = WindSettings {
                direction: Vec3::new(v[4], v[5], v[6]),
                main_speed: v[7],
                speed_variation: v[8],
                noise_ratio: v[9],
                frequency: v[3],
                persistence: v[2],
                octaves: v[1] as i32,
            };
            let wind = Wind {
                settings,
                direction: settings.direction,
                main_speed: v[7],
                speed_variation: v[8],
                phase: v[10],
                previous_phase: v[10],
            };
            let actual = [
                simplex_noise(v[0]),
                fractal_noise(
                    settings.octaves,
                    settings.persistence,
                    settings.frequency,
                    v[0],
                ),
            ]
            .into_iter()
            .chain(wind.velocity(Vec3::new(v[11], v[12], v[13])).to_array());
            for (field, (actual, expected)) in actual.zip(v[14..].iter().copied()).enumerate() {
                assert!(
                    (actual - expected).abs() <= 0.00003 * expected.abs().max(1.),
                    "wind case{case} field{field}: {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn center_wind_normalizes_speed_scaled_direction_for_both_offsets() {
        let mut wind = Wind::new(WindSettings {
            direction: Vec3::new(0., 3., 4.),
            main_speed: -2.,
            speed_variation: 0.,
            ..WindSettings::default()
        });
        wind.advance(1.);
        let [previous, current] = wind.center_wind(Vec3::new(5., 6., 7.));
        // Negative speed flips the unit direction; w is the magnitude.
        for record in [previous, current] {
            for (actual, expected) in record.into_iter().zip([0., -0.6, -0.8, 2.]) {
                assert!((actual - expected).abs() < 1e-6, "{record:?}");
            }
        }
        wind.set_speeds(0., 0.);
        assert_eq!(wind.center_wind(Vec3::ZERO)[1], [0.; 4]);
    }

    #[test]
    fn post_load_map_prefers_started_higher_priority_triggers() {
        use crate::gameplay::trigger_volume::TriggerVolume;
        let cube = |lo: f32, hi: f32| TriggerVolume {
            bounds: [[lo; 3], [hi; 3]],
            hulls: vec![(0..6)
                .map(|face| {
                    let mut plane = [0.; 4];
                    plane[face / 2] = if face % 2 == 0 { 1. } else { -1. };
                    plane[3] = if face % 2 == 0 { -hi } else { lo };
                    plane
                })
                .collect()],
            hull_vertices: vec![Vec::new()],
        };
        let settings = |main_speed| WindSettings {
            main_speed,
            ..WindSettings::default()
        };
        let source = |speed, priority, lo, hi, started| TriggerWindSource {
            entity: String::new(),
            settings: settings(speed),
            priority,
            volume: cube(lo, hi),
            started,
        };
        let manager = WindManager::post_load_map(
            true,
            vec![
                source(2., 0, -10., 10., true),
                source(3., 5, -1., 1., true),
                source(4., 9, -1., 1., false),
                source(5., 0, -10., 10., true),
            ],
            &[[0.; 3], [5.; 3], [50.; 3]],
        );
        assert_eq!(manager.assignment(0), Some(WindOwner::Trigger(1)));
        assert_eq!(manager.assignment(1), Some(WindOwner::Trigger(0)));
        assert_eq!(manager.assignment(2), Some(WindOwner::World));
        assert_eq!(manager.ambiguous_centers, 1);
        let world = Wind::new(settings(1.));
        assert_eq!(
            manager.center_wind(Some(&world), 2, [0.; 3]),
            Some(world.center_wind(Vec3::ZERO))
        );
        let none = WindManager::post_load_map(false, Vec::new(), &[[0.; 3]]);
        assert_eq!(none.center_wind(Some(&world), 0, [0.; 3]), None);
    }

    #[test]
    #[ignore = "requires installed Dishonored 2 and artifacts/wind-manager-centers.txt \
                (analysis/probe_wind_manager_centers.py on a loaded madinventor_p)"]
    fn installed_trigger_winds_partition_live_wind_centers() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        let level =
            crate::assets::level::LevelResources::resolve(&catalog, "madinventor_p").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let world = crate::entities::wind::world_wind(&document, |name| {
            catalog
                .read(name)
                .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        })
        .unwrap();
        let triggers = trigger_wind_sources(&catalog, &document).unwrap();
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/artifacts/wind-manager-centers.txt"
        ))
        .unwrap();
        let mut lines = text.lines().peekable();
        let world_pointer = lines.next().unwrap().strip_prefix("WORLD ").unwrap().to_owned();
        let mut sources = HashMap::new();
        while let Some(source) = lines.peek().and_then(|line| line.strip_prefix("SOURCE ")) {
            let f: Vec<&str> = source.split_whitespace().collect();
            let p = |i: usize| f[i].parse::<f32>().unwrap();
            let settings = WindSettings {
                direction: Vec3::new(p(1), p(2), p(3)),
                main_speed: p(4),
                speed_variation: p(5),
                noise_ratio: p(6),
                frequency: p(7),
                persistence: p(8),
                octaves: f[9].parse().unwrap(),
            };
            sources.insert(f[0].to_owned(), settings);
            lines.next();
        }
        let (centers, live): (Vec<[f32; 3]>, Vec<String>) = lines
            .map(|line| {
                let f: Vec<&str> = line.split_whitespace().collect();
                let p = |i: usize| f[i].parse::<f32>().unwrap();
                ([p(0), p(1), p(2)], f[3].to_owned())
            })
            .unzip();
        let manager = WindManager::post_load_map(world.is_some(), triggers, &centers);
        let mut live_to_ours = HashMap::new();
        let mut ours_to_live = HashMap::new();
        let mut mismatches = 0;
        for (center, pointer) in live.iter().enumerate() {
            let ours = manager.assignment(center);
            assert_eq!(
                pointer == &world_pointer,
                ours == Some(WindOwner::World),
                "center {center} {:?}",
                centers[center]
            );
            let settings = match ours.unwrap() {
                WindOwner::World => world.unwrap(),
                WindOwner::Trigger(index) => manager.triggers[index].settings,
            };
            assert_eq!(settings, sources[pointer], "center {center} settings");
            let a = *live_to_ours.entry(pointer.clone()).or_insert(ours);
            let b = ours_to_live.entry(ours).or_insert_with(|| pointer.clone()).clone();
            mismatches += usize::from(a != ours || &b != pointer);
        }
        assert_eq!(mismatches, 0);
        assert_eq!(manager.ambiguous_centers, 0);
        println!(
            "{} centers, {} trigger winds, {} distinct live owners",
            centers.len(),
            manager.triggers.len(),
            live_to_ours.len()
        );
    }

    #[test]
    #[ignore = "requires installed Dust District archives"]
    fn installed_dust_shutters_swing_in_the_world_wind() {
        let catalog = crate::archive::Catalog::open(
            &crate::assets::installation::game_base().unwrap(),
        )
        .unwrap();
        let level = crate::assets::level::LevelResources::resolve(&catalog, "dust_p_lowchaos").unwrap();
        let bytes = level.read_entities(&catalog).unwrap().unwrap();
        let document = crate::entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
        let read = |p: &str| {
            catalog
                .read(p)
                .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                .map_err(|e| e.to_string())
        };
        let world_wind = Wind::new(crate::entities::wind::world_wind(&document, read).unwrap().unwrap());
        let triggers = trigger_wind_sources(&catalog, &document).unwrap();
        let names: Vec<String> = triggers.iter().map(|t| t.entity.clone()).collect();
        let manager = WindManager::post_load_map(true, triggers, &[]);
        let mut placements = crate::entities::render_placements(&document);
        placements.extend(crate::entities::placed_models_with_inheritance(&document, read).unwrap());
        let recipes =
            crate::entities::machinery::recipes::load(&catalog, &document, &placements).unwrap();
        let mut world =
            crate::gameplay::machinery_world::MachineryWorld::from_recipes(&recipes).unwrap();
        let shutters: Vec<usize> = (0..world.bodies.len())
            .filter(|&i| world.bodies[i].wind.is_some())
            .collect();
        assert_eq!(shutters.len(), 10);
        // Live dust_p_lowchaos capture (analysis/probe_wind_rigid_bodies.py):
        // every shutter lists fosterbldg_shutter then vista; the lowest one
        // (spawn z -26.47) also touches vistatun, first.
        for &i in &shutters {
            world.bodies[i].update_wind_triggers(&manager.trigger_shapes);
            let listed: Vec<&str> = world.bodies[i].wind.as_ref().unwrap().triggers.iter()
                .map(|t| names[*t].as_str()).collect();
            let low = world.bodies[i].mass_center()[2] < -24.;
            let mut expected = vec!["audio_area_volume_fosterbldg_shutter", "audio_area_volume_vista"];
            if low {
                expected.insert(0, "audio_area_volume_vistatun");
            }
            assert_eq!(listed, expected);
        }
        let start: Vec<_> = shutters.iter().map(|&i| world.bodies[i].motion.pose.orientation).collect();
        let mut world_wind = world_wind;
        let mut swing = vec![0f32; shutters.len()];
        for _ in 0..600 {
            world_wind.advance(1. / 60.);
            for &i in &shutters {
                let body = &mut world.bodies[i];
                let wind = match body.wind.as_ref().unwrap().triggers.first() {
                    Some(&t) => &manager.triggers[t],
                    None => &world_wind,
                };
                let velocity = wind.velocity(Vec3::from_array(body.mass_center()));
                body.apply_wind(velocity.to_array(), 1. / 60.);
            }
            world.step([1. / 60.; 5]);
            for &i in &shutters {
                world.bodies[i].update_wind_triggers(&manager.trigger_shapes);
            }
            for (k, &i) in shutters.iter().enumerate() {
                let q = world.bodies[i].motion.pose.orientation;
                let d = (0..4).map(|j| q[j] * start[k][j]).sum::<f32>().abs().min(1.);
                swing[k] = swing[k].max(2. * d.acos().to_degrees());
            }
        }
        for &i in &shutters {
            let body = &world.bodies[i];
            assert!(body.motion.pose.center.iter().chain(&body.motion.pose.orientation).all(|v| v.is_finite()));
        }
        println!("max swing degrees {swing:?}; triggers {:?}; anchor error {}",
            shutters.iter().map(|&i| world.bodies[i].wind.as_ref().unwrap().triggers.clone()).collect::<Vec<_>>(),
            world.maximum_anchor_error());
        assert!(world.maximum_anchor_error() < 0.05);
    }

}