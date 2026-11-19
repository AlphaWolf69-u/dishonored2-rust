//! Startup execution for autonomous motion scenes. Cinematic/non-motion tracks
//! and weather-event production remain separate integrations.
use super::event_graph::GraphInputs;
use super::machinery_world::MachineryWorld;
use super::motion_curve::angle_basis;
use crate::entities::{
    scene_motion::{atom, retain_clip, Graph, MotionClip, Node},
    PlacedModel,
};
use std::{collections::{HashMap, HashSet, VecDeque}, sync::Arc};

use crate::random::MwcRandom;

struct Playback {
    node: u32,
    placement: usize,
    clip: Arc<crate::assets::midnight_scene::MotionScene>,
    group: usize,
    rate: f32,
    time_ms: f64,
    position: [f32; 3],
    basis: [[f32; 3]; 3],
}
impl Playback {
    fn delta(&self) -> [[f32; 4]; 3] {
        scene_delta(&self.clip, self.group, self.time_ms, self.position, self.basis)
    }
}
fn scene_delta(clip: &MotionClip, group: usize, time_ms: f64, origin: [f32; 3], basis: [[f32; 3]; 3]) -> [[f32; 4]; 3] {
        let (position, angles) = clip.groups[group]
            .curve
            .sample(time_ms as i32);
        let local = angle_basis(angles);
        //817060 captures the owner's reference frame;816710 composes the
        //unwrapped Euler curve with that frame. Publish a WORLD delta so all
        //render representations retain their authored scale and shear.
        let rotation: [[f32; 3]; 3] = std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                (0..3)
                    .map(|a| {
                        (0..3)
                            .map(|b| basis[a][r] * local[b][a] * basis[b][c])
                            .sum::<f32>()
                    })
                    .sum()
            })
        });
        std::array::from_fn(|r| {
            std::array::from_fn(|c| {
                if c < 3 {
                    rotation[r][c]
                } else {
                    origin[r] + (0..3).map(|a| basis[a][r] * position[a]).sum::<f32>()
                        - (0..3)
                            .map(|a| rotation[r][a] * origin[a])
                            .sum::<f32>()
                }
            })
        })
}

struct RunningGraph {
    graph: Graph,
    /// Only dependencies of supported scene actions belong to this runtime.
    /// LoadingEnd also fans out to unrelated autosaves, spawners and AI nodes.
    relevant: HashSet<u32>,
    random: HashMap<u32, f32>,
    variables: HashMap<String, f32>,
    playing: Vec<Playback>,
    animated: Vec<AnimatedPlayback>,
    motor_controls: Vec<MotorControl>,
}

struct AnimatedPlayback {
    node: u32,
    placement: usize,
    group: usize,
    clip: std::sync::Arc<crate::assets::midnight_scene::AnimatedScene>,
    time_ms: f64,
    position: [f32; 3],
    basis: [[f32; 3]; 3],
}

/// The level-scoped scene-skin owner currently admits autonomous loops that
/// start once with the level and cannot receive later stop/replay requests.
/// Dynamic scenes need on-demand acquisition and release when their instance
/// starts and ends, so they remain with the general scene runtime.
fn autonomous_scene_action(graph: &Graph, node: &Node) -> bool {
    graph.animated_clips.contains_key(atom(&node.params, "m_declMdnt").unwrap_or(""))
        && crate::entities::scene_motion::autonomous_scene_action(&graph.nodes, node)
}

pub(crate) trait MotorControls {
    fn get(&self, id: &str) -> Result<Option<(bool, f32, f32, f32)>, String>;
    fn set(&mut self, id: &str, enabled: Option<bool>, position: Option<f32>,
        velocity: Option<f32>, acceleration: Option<f32>) -> Result<(), String>;
}

impl MotorControls for MachineryWorld {
    fn get(&self, id: &str) -> Result<Option<(bool, f32, f32, f32)>, String> {
        Ok(self.motor_parameters(id))
    }
    fn set(&mut self, id: &str, enabled: Option<bool>, position: Option<f32>,
        velocity: Option<f32>, acceleration: Option<f32>) -> Result<(), String> {
        self.set_motor_parameters(id, enabled, position, velocity, acceleration);
        Ok(())
    }
}

struct MotorControl {
    node: u32,
    entity: String,
    enabled: bool,
    state_pending: bool,
    remaining: f32,
    position: Option<f32>,
    velocity: Option<f32>,
    acceleration: Option<f32>,
}
impl MotorControl {
    fn advance(&mut self, seconds: f32, world: &mut impl MotorControls) -> Result<bool, String> {
        if !self.enabled {
            world.set(&self.entity, Some(false), None, None, None)?;
            return Ok(true);
        }
        let Some((_, p, v, a)) = world.get(&self.entity)? else {
            return Ok(true);
        };
        let fraction = if self.remaining <= 0. {
            1.
        } else {
            (seconds / self.remaining).min(1.)
        };
        //52f3c0: interpolate from CURRENT parameters using the remaining time.
        //Values below141ab2294 are zeroed before the two multiplications.
        let blend = |current: f32, target: f32| {
            let clean = |x: f32| {
                if x.abs() <= f32::from_bits(0x219392ef) {
                    0.
                } else {
                    x
                }
            };
            fraction * clean(target) + (1. - fraction) * clean(current)
        };
        world.set(
            &self.entity,
            self.state_pending.then_some(self.enabled),
            self.position.map(|t| blend(p, t)),
            self.velocity.map(|t| blend(v, t)),
            self.acceleration.map(|t| blend(a, t)),
        )?;
        self.state_pending = false;
        self.remaining -= seconds;
        Ok(fraction >= 1.)
    }
}
impl GraphInputs for RunningGraph {
    fn graph_nodes(&self) -> &[Node] {
        &self.graph.nodes
    }
    fn graph_owner(&self) -> &str {
        &self.graph.owner
    }
    fn random_sample(&self, node: u32) -> Option<f32> {
        self.random.get(&node).copied()
    }
    fn named_value(&self, name: &str) -> Result<Option<f32>, String> {
        Ok(self.variables.get(name).copied())
    }
}
impl RunningGraph {
    fn bindings(&self, n: &Node, clip: &MotionClip) -> Result<Vec<(usize, String)>, String> {
        let (id, _) = self
            .input(n, "m_midnightParams")
            .ok_or("scene parameters have no source")?;
        let source = self.node(id)?;
        if source.class != "arkKisculeNodeOpMidnightParametersControl" {
            return Err(format!(
                "unsupported scene parameter producer {}",
                source.class
            ));
        }
        if atom(&source.params, "m_declMdnt") != atom(&n.params, "m_declMdnt") {
            return Err("scene parameter declaration mismatch".into());
        }
        let mut result = Vec::new();
        for (index, g) in clip.groups.iter().enumerate() {
            if let Some((id, port)) = self.input(source, &g.name) {
                for entity in self.entities(id, port)? {
                    result.push((index, entity));
                }
            } else {
                // Runtime groups are input variables, not m_previewSetup.
                return Err(format!("group {} has no runtime entity binding", g.name));
            }
        }
        Ok(result)
    }
    fn emit(&self, n: &Node, name: &str, q: &mut VecDeque<(u32, u32)>) {
        for p in &n.ports {
            if p.name == name && p.kind.starts_with("Output") {
                q.extend(
                    p.links
                        .iter()
                        .copied()
                        .filter(|(id, _)| self.relevant.contains(id)),
                );
            }
        }
    }
    fn execute(&mut self, mut q: VecDeque<(u32, u32)>, placements: &[PlacedModel]) -> Vec<String> {
        static OBSERVATION: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
        let (copy, trace) = *OBSERVATION.get_or_init(|| (
            std::env::var_os("DH2_DEBUG_CLONE_MOTION_NODES").is_some_and(|value| value == "1"),
            std::env::var_os("DH2_DEBUG_MOTION_DISPATCH").is_some_and(|value| value == "1"),
        ));
        let started = trace.then(crate::loading::thread_cycles).flatten();
        let mut diagnostics = Vec::new();
        let mut steps = 0;
        while let Some((id, port)) = q.pop_front() {
            steps += 1;
            if steps > 4096 {
                diagnostics.push("startup graph did not settle".into());
                break;
            }
            let result = (|| -> Result<(), String> {
                let n = self.graph.nodes.iter().find(|node| node.id == id)
                    .ok_or_else(|| format!("missing node {id}"))?;
                let owned = copy.then(|| n.clone());
                let n = owned.as_ref().unwrap_or(n);
                let p = n
                    .ports
                    .iter()
                    .find(|p| p.id == port)
                    .ok_or("missing event port")?;
                match n.class.as_str() {
                    "arkKisculeNodeActionTestBool" => {
                        //7967f0 reads the connected bool and dispatches one branch.
                        let value = if let Some((id, port)) = self.input(n, "m_value") {
                            self.number(id, port, 0)? != 0.
                        } else {
                            atom(&n.params, "m_value") == Some("true")
                        };
                        self.emit(n, if value { "True" } else { "False" }, &mut q);
                    }
                    "arkKisculeNodeActionAngularPhysicsMotorControl" => {
                        let entities = if let Some((id, port)) = self.input(n, "m_entity") {
                            self.entities(id, port)?
                        } else {
                            atom(&n.params, "m_entity")
                                .filter(|s| !s.is_empty())
                                .map(|s| vec![s.into()])
                                .unwrap_or_default()
                        };
                        let enabled =
                            atom(&n.params, "m_setMotorState").unwrap_or("true") == "true";
                        let selected = |key| enabled && atom(&n.params, key) == Some("true");
                        let radians = f32::from_bits(0x3c8efa35);
                        let position = if selected("m_setAngularTargetPosition") {
                            Some(self.number_input(n, "m_angularTargetPosition", 0., 0)? * radians)
                        } else {
                            None
                        };
                        let velocity = if selected("m_setAngularTargetVelocity") {
                            Some(self.number_input(n, "m_angularTargetVelocity", 0., 0)? * radians)
                        } else {
                            None
                        };
                        let acceleration = if selected("m_setAngularTargetAcceleration") {
                            Some(
                                self.number_input(n, "m_angularTargetAcceleration", 0., 0)?
                                    .max(0.)
                                    * radians,
                            )
                        } else {
                            None
                        };
                        let remaining =
                            if position.is_some() || velocity.is_some() || acceleration.is_some() {
                                self.number_input(n, "m_updateDuration", 0., 0)? / 1000.
                            } else {
                                0.
                            };
                        self.motor_controls.retain(|c| c.node != id);
                        for entity in entities {
                            self.motor_controls.push(MotorControl {
                                node: id,
                                entity,
                                enabled,
                                state_pending: true,
                                remaining,
                                position,
                                velocity,
                                acceleration,
                            });
                        }
                    }
                    "arkKisculeNodeRelay" => self.emit(n, "Out", &mut q),
                    "arkKisculeNodeNamedVariableFloat" if p.name == "Set" => {
                        let value = self.number_input(n, "m_value", 0., 0)?;
                        let name = atom(&n.params, "m_variableName").unwrap_or("");
                        self.variables.insert(name.to_owned(), value);
                        for listener in &self.graph.nodes {
                            if listener.class == n.class
                                && atom(&listener.params, "m_variableName") == Some(name)
                            {
                                self.emit(listener, "Updated", &mut q);
                            }
                        }
                    }
                    "arkKisculeNodeActionMidnightKeyframeControl" => {
                        if !self
                            .graph
                            .clips
                            .contains_key(atom(&n.params, "m_declMdnt").unwrap_or(""))
                        {
                            return Ok(());
                        }
                        match p.name.as_str() {
                            "Stop" => {
                                self.playing.retain(|p| p.node != id);
                                self.emit(n, "Stop", &mut q);
                            }
                            "Play" => {
                                if atom(&n.params, "m_playbackType")
                                    .is_some_and(|s| s != "EMPT_DEFAULT")
                                {
                                    return Err("nondefault scene playback".into());
                                }
                                let clip = retain_clip(&self.graph.clips[atom(&n.params, "m_declMdnt").unwrap()]);
                                let rate = self.number_input(n, "m_playRate", 1., 0)?.max(0.001);
                                let time = self.number_input(n, "m_startPosition", 0., 0)?;
                                if time != 0. {
                                    return Err(
                                        "scene seek/reference-frame capture is not integrated"
                                            .into(),
                                    );
                                }
                                let bindings = self.bindings(n, &clip)?;
                                if !self.playing.iter().any(|p| p.node == id) {
                                    for (group, entity) in bindings {
                                        let (placement, model) = placements
                                            .iter()
                                            .enumerate()
                                            .find(|(_, p)| p.id == entity)
                                            .ok_or_else(|| {
                                                format!(
                                                    "scene entity {entity} has no render placement"
                                                )
                                            })?;
                                        self.playing.push(Playback {
                                            node: id,
                                            placement,
                                            clip: retain_clip(&clip),
                                            group,
                                            rate,
                                            time_ms: time as f64,
                                            position: model.position,
                                            basis: model.rotation,
                                        });
                                    }
                                }
                                self.emit(n, "Play", &mut q);
                            }
                            other => return Err(format!("unsupported scene event {other}")),
                        }
                    }
                    "arkKisculeNodeActionMidnightControl" => {
                        if !autonomous_scene_action(&self.graph, n) { return Ok(()); }
                        let Some(clip) = self.graph.animated_clips.get(atom(&n.params, "m_declMdnt").unwrap_or("")).cloned() else { return Ok(()) };
                        match p.name.as_str() {
                            "Stop" => {
                                self.playing.retain(|playback| playback.node != id);
                                self.animated.retain(|playback| playback.node != id);
                                self.emit(n, "Stop", &mut q);
                            }
                            "Play" => {
                                if self.animated.iter().any(|playback| playback.node == id) {
                                    if atom(&n.params, "m_resetOnPlay") == Some("true") {
                                        return Err("scene reset on replay is not integrated".into());
                                    }
                                    return Ok(());
                                }
                                for (group, entity) in self.bindings(n, &clip.motion)? {
                                    let (placement, model) = placements.iter().enumerate().find(|(_, p)| p.id == entity)
                                        .ok_or_else(|| format!("scene entity {entity} has no render placement"))?;
                                    self.animated.push(AnimatedPlayback { node: id, placement, group, clip: clip.clone(),
                                        time_ms: 0., position: model.position, basis: model.rotation });
                                }
                                self.emit(n, "Play", &mut q);
                            }
                            other => return Err(format!("unsupported scene event {other}")),
                        }
                    }
                    other => return Err(format!("unsupported startup node {other}")),
                }
                Ok(())
            })();
            if let Err(e) = result {
                diagnostics.push(format!("{} node {id}: {e}", self.graph.owner));
            }
        }
        if let Some(started) = started {
            let cycles = crate::loading::thread_cycles().unwrap_or(started).saturating_sub(started);
            eprintln!("MOTION_DISPATCH owner={} copied={copy} steps={steps} errors={} cycles={cycles}", self.graph.owner, diagnostics.len());
        }
        diagnostics
    }
}

#[derive(Default)]
pub struct SceneMotion {
    graphs: Vec<RunningGraph>,
    pub diagnostics: Vec<String>,
}
impl SceneMotion {
    pub fn new(graphs: Vec<Graph>, placements: &[PlacedModel], counter: u64) -> Self {
        let mut random = MwcRandom::new(counter);
        let mut result = Self::default();
        for graph in graphs {
            let mut relevant: HashSet<_> = graph
                .nodes
                .iter()
                .filter(|n| {
                    n.class == "arkKisculeNodeActionAngularPhysicsMotorControl"
                        || autonomous_scene_action(&graph, n)
                        || (n.class == "arkKisculeNodeActionMidnightKeyframeControl"
                            && graph
                                .clips
                                .contains_key(atom(&n.params, "m_declMdnt").unwrap_or("")))
                })
                .map(|n| n.id)
                .collect();
            loop {
                let previous = relevant.len();
                for n in &graph.nodes {
                    if !relevant.contains(&n.id) {
                        continue;
                    }
                    for p in n.ports.iter().filter(|p| p.kind.starts_with("Input")) {
                        relevant.extend(p.links.iter().map(|(n, _)| *n));
                    }
                    if n.class == "arkKisculeNodeNamedVariableFloat" {
                        for writer in &graph.nodes {
                            if writer.class == n.class
                                && atom(&writer.params, "m_variableName")
                                    == atom(&n.params, "m_variableName")
                            {
                                relevant.insert(writer.id);
                            }
                        }
                    }
                }
                if relevant.len() == previous {
                    break;
                }
            }
            let mut run = RunningGraph {
                graph,
                relevant,
                random: HashMap::new(),
                variables: HashMap::new(),
                playing: Vec::new(),
                animated: Vec::new(),
                motor_controls: Vec::new(),
            };
            for n in &run.graph.nodes {
                if n.class == "arkKisculeNodeRandomFloat" {
                    let min = atom(&n.params, "m_min")
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0.);
                    let max = atom(&n.params, "m_max")
                        .unwrap_or("1")
                        .parse()
                        .unwrap_or(1.);
                    run.random.insert(n.id, random.between(min, max));
                }
            }
            let mut q = VecDeque::new();
            for n in &run.graph.nodes {
                if n.class == "arkKisculeNodeLoadingEnd" {
                    run.emit(n, "LevelLoadedFirstTime", &mut q);
                }
            }
            result.diagnostics.extend(run.execute(q, placements));
            result.graphs.push(run);
        }
        result.report_resources();
        result
    }
    /// Observe actual initialized owners, without advancing gameplay or
    /// constructing a synthetic scene. Logical payload excludes hash buckets.
    fn report_resources(&self) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
        let mut unique = HashSet::new();
        let mut leases = 0;
        let mut payload_bytes = 0;
        let mut digest = blake3::Hasher::new();
        let mut poses = blake3::Hasher::new();
        let mut rates = Vec::new();
        let mut record = |clip: &Arc<crate::assets::midnight_scene::MotionScene>| {
            leases += 1;
            if unique.insert(Arc::as_ptr(clip)) {
                payload_bytes += std::mem::size_of::<MotionClip>()
                    + clip.groups.capacity() * std::mem::size_of::<crate::entities::scene_motion::MotionGroup>();
                for group in &clip.groups {
                    payload_bytes += group.name.capacity()
                        + group.curve.keys.capacity() * std::mem::size_of::<crate::entities::scene_motion::CurveKey>();
                }
            }
        };
        for graph in &self.graphs {
            digest.update(format!("{:?}", (&graph.graph.owner, &graph.graph.nodes)).as_bytes());
            let mut clips: Vec<_> = graph.graph.clips.iter().collect();
            clips.sort_unstable_by(|a, b| a.0.cmp(b.0));
            for (name, clip) in clips {
                record(clip);
                digest.update(format!("{name}:{clip:?}").as_bytes());
            }
            for playback in &graph.playing {
                record(&playback.clip);
                // Rate is sampled from the live performance-counter RNG at
                // scene creation. Report it separately; it is not a source
                // identity or initial pose, and must not be frozen for a check.
                rates.push(playback.rate);
                poses.update(format!("{:?}", (playback.node, playback.placement, playback.group,
                    playback.time_ms, playback.position, playback.basis, playback.delta())).as_bytes());
            }
        }
        let tag_context_bytes = crate::platform::allocation_tags::live_bytes("Motion scene resources");
        eprintln!("MOTION_SCENE_STORAGE graphs={} playing={} leases={leases} unique_clips={} logical_payload_bytes={payload_bytes} definition_signature={} initial_pose_signature={} runtime_rates={rates:?} tag_context_bytes={tag_context_bytes:?}",
            self.graphs.len(), self.playing_count(), unique.len(), digest.finalize(), poses.finalize());
    }
    pub fn playing_count(&self) -> usize {
        self.graphs.iter().map(|g| g.playing.len() + g.animated.len()).sum()
    }
    pub fn advance_motors(&mut self, seconds: f32, world: &mut MachineryWorld) {
        self.advance_motor_controls(seconds, world).expect("ambient motor operations are infallible");
    }
    pub(crate) fn advance_motor_controls(&mut self, seconds: f32, world: &mut impl MotorControls) -> Result<(), String> {
        for graph in &mut self.graphs {
            let mut result = Ok(());
            graph.motor_controls.retain_mut(|control| {
                if result.is_err() { return true; }
                match control.advance(seconds, world) {
                    Ok(done) => !done,
                    Err(error) => { result = Err(error); true }
                }
            });
            result?;
        }
        Ok(())
    }
    pub fn animated_groups(&self) -> impl Iterator<Item = (usize, &std::sync::Arc<crate::assets::midnight_scene::AnimatedScene>, usize, i32)> {
        self.graphs.iter().flat_map(|graph| graph.animated.iter().filter_map(move |animated| {
            let uid = animated.clip.motion.groups[animated.group].uid;
            animated.clip.animations.get(&uid)?;
            Some((animated.placement, &animated.clip, animated.group, animated.time_ms as i32))
        }))
    }
    pub fn notify(&mut self, event: &str, placements: &[PlacedModel]) {
        for graph in &mut self.graphs {
            let mut q = VecDeque::new();
            for n in &graph.graph.nodes {
                if n.class == "arkKisculeNodeGetCustomEvent"
                    && atom(&n.params, "m_eventName") == Some(event)
                    && atom(&n.params, "m_enabled") != Some("false")
                {
                    graph.emit(n, "Received", &mut q);
                }
            }
            self.diagnostics.extend(graph.execute(q, placements));
        }
    }
    pub fn step(&mut self, seconds: f32, mut publish: impl FnMut(usize, [[f32; 4]; 3])) {
        for g in &mut self.graphs {
            for p in &mut g.animated {
                p.time_ms = (p.time_ms + seconds as f64 * 1000.).rem_euclid(p.clip.motion.length_ms as f64);
                if p.clip.motion_groups.contains(&p.clip.motion.groups[p.group].uid) {
                    publish(p.placement, scene_delta(&p.clip.motion, p.group, p.time_ms, p.position, p.basis));
                }
            }
            for p in &mut g.playing {
                p.time_ms = (p.time_ms + seconds as f64 * 1000. * p.rate as f64)
                    .rem_euclid(p.clip.length_ms as f64);
                publish(p.placement, p.delta());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_motor_scripts_apply_startup_and_storm_targets() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        for name in ["dust_p_lowchaos", "dust_p_highchaos"] {
            let bytes = catalog
                .read(&format!("maps/campaign/dust/{name}.entities"))
                .unwrap()
                .unwrap();
            let doc = crate::entities::parse(&crate::entities::declaration_text(&bytes)).unwrap();
            let mut placements = crate::entities::render_placements(&doc);
            placements.extend(
                crate::entities::placed_models_with_inheritance(&doc, |p| {
                    catalog
                        .read(p)
                        .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                        .map_err(|e| e.to_string())
                })
                .unwrap(),
            );
            let recipes =
                crate::entities::machinery::recipes::load(&catalog, &doc, &placements).unwrap();
            let mut world = MachineryWorld::from_recipes(&recipes).unwrap();
            let mut tested = 0;
            for windmill in &recipes.windmills {
                if let Some((_, _, velocity, _)) = world.motor_parameters(&windmill.constraint) {
                    assert!((velocity.to_degrees() - windmill.speed_degrees).abs() < 0.001);
                    assert!(world.set_windmill_speed(&windmill.entity, 800.));
                    assert!(
                        (world
                            .motor_parameters(&windmill.constraint)
                            .unwrap()
                            .2
                            .to_degrees()
                            - 800.)
                            .abs()
                            < 0.001
                    );
                    assert!(world.set_windmill_speed(&windmill.entity, -1.));
                    assert!(
                        (world
                            .motor_parameters(&windmill.constraint)
                            .unwrap()
                            .2
                            .to_degrees()
                            - windmill.speed_degrees)
                            .abs()
                            < 0.001
                    );
                    tested += 1;
                }
            }
            assert!(tested > 0, "{name}: no generator-driven motor initialized");
            // Exercise the motor-control graph. Weather production and the
            // embedded scene-rate interpolation graph have separate owners.
            let graphs = crate::entities::scene_motion::load(&catalog, &doc)
                .unwrap()
                .into_iter()
                .filter(|g| {
                    g.nodes
                        .iter()
                        .any(|n| n.class == "arkKisculeNodeActionAngularPhysicsMotorControl")
                })
                .collect();
            let mut scripts = SceneMotion::new(graphs, &placements, 0);
            let id = "env_vista_constraint_rotor_5";
            assert!((world.motor_parameters(id).unwrap().2.to_degrees() - 360.).abs() < 0.001);
            for (event, seconds, target) in [
                (None, 5.1, 35.),
                (Some("duststorm_wind_start"), 2.1, 90.),
                (Some("duststorm_stop"), 5.1, 35.),
            ] {
                if let Some(event) = event {
                    scripts.notify(event, &placements);
                }
                for _ in 0..(seconds * 60.) as usize {
                    scripts.advance_motors(1. / 60., &mut world);
                    world.step([1. / 60.; 2]);
                }
                let (enabled, _, velocity, _) = world.motor_parameters(id).unwrap();
                assert!(enabled);
                assert!(
                    (velocity.to_degrees() - target).abs() < 0.001,
                    "{name} {event:?}: {}",
                    velocity.to_degrees()
                );
                assert!(world.maximum_anchor_error() < 0.1);
                assert!(scripts.diagnostics.is_empty(), "{:?}", scripts.diagnostics);
            }
        }
    }
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_autonomous_scene_motion_audit() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        let mut total = 0;
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let doc = crate::entities::parse(&crate::entities::declaration_text(&bytes)).unwrap();
            let mut placements = crate::entities::render_placements(&doc);
            placements.extend(
                crate::entities::placed_models_with_inheritance(&doc, |p| {
                    catalog
                        .read(p)
                        .map(|b| b.map(|b| crate::entities::declaration_text(&b).into_owned()))
                        .map_err(|e| e.to_string())
                })
                .unwrap(),
            );
            let graphs = crate::entities::scene_motion::load(&catalog, &doc).unwrap();
            let mut scenes = SceneMotion::new(graphs, &placements, 0);
            println!(
                "{path}: {} playing groups; {:?}",
                scenes.playing_count(),
                scenes.diagnostics
            );
            assert!(
                scenes.diagnostics.is_empty(),
                "{path}: {:?}",
                scenes.diagnostics
            );
            total += scenes.playing_count();
            for _ in 0..10 {
                scenes.step(1. / 60., |_, delta| {
                    assert!(delta.iter().flatten().all(|v| v.is_finite()))
                });
            }
        }
        assert!(total >= 6);
    }
    #[test]
    fn motion_reference_frame_matches_installed_composition() {
        use crate::entities::scene_motion::{CurveKey, Interpolation, MotionCurve, MotionGroup};
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/motion_reference_frame.txt")
            .lines()
            .filter(|s| !s.starts_with('#') && !s.is_empty())
        {
            let w: Vec<_> = line
                .split_whitespace()
                .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
                .collect();
            let key = CurveKey {
                time_ms: 0,
                position: w[12..15].try_into().unwrap(),
                orientation: w[15..18].try_into().unwrap(),
                tangent_in: [0.; 3],
                tangent_out: [0.; 3],
                angle_in: [0.; 3],
                angle_out: [0.; 3],
            };
            let playback = Playback {
                node: 0,
                placement: 0,
                rate: 1.,
                time_ms: 0.,
                position: w[..3].try_into().unwrap(),
                basis: std::array::from_fn(|c| std::array::from_fn(|r| w[3 + c * 3 + r])),
                group: 0,
                clip: crate::assets::midnight_scene::SceneView::detached(MotionClip {
                    length_ms: 12000,
                    groups: vec![MotionGroup {
                        uid: 1,
                        name: String::new(),
                        curve: MotionCurve {
                            interpolation: Interpolation::Linear,
                            keys: vec![key],
                        },
                    }],
                }),
            };
            let delta = playback.delta();
            for r in 0..3 {
                let p = delta[r][3]
                    + (0..3)
                        .map(|c| delta[r][c] * playback.position[c])
                        .sum::<f32>();
                assert!((p - w[18 + r]).abs() < 0.001, "frame{count} position{r}");
                for c in 0..3 {
                    let v = (0..3)
                        .map(|i| delta[r][i] * playback.basis[c][i])
                        .sum::<f32>();
                    assert!(
                        (v - w[21 + c * 3 + r]).abs() < 0.00001,
                        "frame{count} basis{c},{r}: {v} vs {}",
                        w[21 + c * 3 + r]
                    );
                }
            }
            count += 1;
        }
        assert_eq!(count, 128);
    }
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_dust_scene_loops_start_and_move_vista_models() {
        let catalog =
            crate::archive::Catalog::open(&crate::assets::installation::game_base().unwrap())
                .unwrap();
        for name in ["dust_p_highchaos", "dust_p_lowchaos"] {
            let bytes = catalog
                .read(&format!("maps/campaign/dust/{name}.entities"))
                .unwrap()
                .unwrap();
            let doc = crate::entities::parse(&crate::entities::declaration_text(&bytes)).unwrap();
            let placements = crate::entities::render_placements(&doc);
            let graphs = crate::entities::scene_motion::load(&catalog, &doc).unwrap();
            println!("{name}: {} eligible graphs", graphs.len());
            let mut untriggered = graphs.clone();
            for graph in &mut untriggered {
                for node in &mut graph.nodes {
                    if node.class == "arkKisculeNodeLoadingEnd" {
                        for port in &mut node.ports {
                            port.links.clear();
                        }
                    }
                }
            }
            assert_eq!(
                SceneMotion::new(untriggered, &placements, 0).playing_count(),
                0,
                "a scene reference alone must not start playback"
            );
            let mut scenes = SceneMotion::new(graphs, &placements, 0);
            println!(
                "{name}: {} playing; diagnostics {:?}",
                scenes.playing_count(),
                scenes.diagnostics
            );
            assert!(scenes.diagnostics.is_empty());
            assert!(scenes.playing_count() >= 3);
            let mut changes = HashMap::new();
            for _ in 0..60 {
                scenes.step(1. / 60., |index, delta| {
                    changes.insert(index, delta);
                });
            }
            for (index, delta) in changes {
                println!("motion {} {delta:?}", placements[index].id);
                assert!(delta.iter().flatten().all(|v| v.is_finite()));
                assert!(delta.iter().enumerate().any(|(r, row)| row[..3]
                    .iter()
                    .enumerate()
                    .any(|(c, v)| (*v - if r == c { 1. } else { 0. }).abs() > 0.05)));
                // Rigid rotation keeps the entity origin at its authored place.
                for r in 0..3 {
                    let moved = delta[r][3]
                        + (0..3)
                            .map(|c| delta[r][c] * placements[index].position[c])
                            .sum::<f32>();
                    assert!((moved - placements[index].position[r]).abs() < 0.001);
                }
            }
        }
    }
}
