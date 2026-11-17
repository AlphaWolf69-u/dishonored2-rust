//! Midnight scene playback on the player body: the manager's time step
//! (FUN_1407d2da0, integer ms with a float carry), the CONDMD6 graph walk
//! (FUN_1407ccdc0), the anim track's clip schedule and per-frame write
//! (FUN_1407e2860 / FUN_1407e27b0, weights FUN_14018fe90), EMTT_EVENT point
//! keys (FUN_1407dfdd0), the body tree's midnight node (FUN_1406fea90:
//! clips over the loco pose, snapshot blend FUN_1406fe490) and its root
//! motion (FUN_1406ffde0).
//!
//! The cast (FUN_1408542b0) puts PlayerBody / PlayerCamera on the player
//! entity (mask bit 1) and PlayerLeftArm / PlayerRightArm on the arm
//! entities (bits 2 and 4) and actual equipment entities (bits 8 and 16).
//! Each group writes its own tree's midnight node. SFX tracks retain the
//! actual cast entity's emitter through scheduling and interruption.
use super::animation::{blend_add, blend_lerp, sample, Pose};
use crate::edge_animation::EdgeAnimation;
use crate::edge_skeleton::JointRestPose;
use crate::entities::midnight::{FloatCurve, MidnightScene, Next};
use crate::gameplay::sound::{Emitter, Request as SoundRequest};
use bevy_math::{Mat3, Quat, Vec3};
use std::sync::Arc;
mod animation_sounds;
pub use animation_sounds::{AnimationSound, AnimationSoundBank, AnimationSoundReceipt,AnimationEffectReceipt};

/// A scene decl with its anim names resolved to body clips.
#[derive(Debug)]
pub struct PreparedScene {
    pub decl: Arc<crate::assets::midnight_scene::PlayerScene>,
    /// Per (scene index, key index) of the player group: the body clip.
    pub clips: Vec<Vec<usize>>,
    /// The same for the PlayerLeftArm / PlayerRightArm groups: arm clips.
    pub arm_clips: [Vec<Vec<usize>>; 2],
    /// The actual left/right equipment entity trees (8542b0 bits 8/16).
    pub equipment_clips: [Vec<Vec<usize>>; 2],
    /// SFX keys resolved to installed event IDs and their cast audio owners.
    pub sounds: Vec<Vec<PreparedSound>>,
    pub animation_sounds: AnimationSoundBank,
}

#[derive(Clone, Debug)]
pub struct PreparedSound {
    pub key: crate::entities::midnight::SoundKey,
    pub event: u32,
    pub emitter: Emitter,
}

/// Resolve each cast group's SFX against its actual model and the event
/// declaration's engine ID. The same preparation is used by the client and
/// installed-resource diagnostics.
pub fn prepare_sounds(
    decl: &MidnightScene,
    casting: &[(&str, &str, Emitter)],
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<Vec<PreparedSound>>, String> {
    prepare_sounds_using(decl, casting, |name| {
        let resource = format!("generated/decls/soundevent/{name}.decl");
        let source = read(&resource)?.ok_or_else(|| format!("{resource} is absent"))?;
        crate::assets::wwise::declared_event_id(&source).map_err(|e| format!("{resource}: {e}"))
    })
}

/// The production acquisition path shares the same scoped sound-event owner
/// as entity/FX sounds. SFX property copies retain a declaration pointer in
///140188490; scheduled playback here keeps its existing copied engine ID.
pub(crate) fn prepare_sounds_owned(
    catalog: &crate::archive::Catalog,
    decl: &MidnightScene,
    casting: &[(&str, &str, Emitter)],
) -> Result<Vec<Vec<PreparedSound>>, String> {
    if std::env::var("DH2_DEBUG_SEPARATE_MIDNIGHT_SOUND_EVENTS").as_deref() == Ok("1") {
        return prepare_sounds(decl, casting, |path| {
            Ok(catalog.read(path).map_err(|error| error.to_string())?
                .map(crate::entities::owned_declaration_text))
        });
    }
    prepare_sounds_using(decl, casting, |name| {
        crate::assets::wwise::event_resources::acquire_with(catalog, name, |path|
            crate::assets::declaration_texts::read_common_or_catalog(catalog, path))
            .map(|event| event.id)
    })
}

fn prepare_sounds_using(
    decl: &MidnightScene,
    casting: &[(&str, &str, Emitter)],
    mut resolve: impl FnMut(&str) -> Result<u32, String>,
) -> Result<Vec<Vec<PreparedSound>>, String> {
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let mut ids = std::collections::HashMap::new();
    let mut sounds = vec![Vec::new(); decl.scenes.len()];
    for (index, scene) in decl.scenes.iter().enumerate() {
        for group in &scene.groups {
            let Some((_, model, emitter)) = casting.iter().find(|(name, _, _)| Some(*name) == decl.group_name(group.group)) else { continue };
            if !group.specific_models.is_empty() && !group.specific_models.iter().any(|m| m == model) { continue; }
            for key in group.sounds.iter().filter(|key| !key.event.is_empty()) {
                let event = match ids.get(&key.event) {
                    Some(&event) => event,
                    None => {
                        let event = resolve(&key.event)?;
                        if trace { eprintln!("MIDNIGHT_SOUND_RESOLVE reference={} id={event}", key.event); }
                        ids.insert(key.event.clone(), event);
                        event
                    }
                };
                if event != 0 { sounds[index].push(PreparedSound { key: key.clone(), event, emitter: *emitter }); }
            }
        }
    }
    Ok(sounds)
}

struct ScheduledSound {
    sound: PreparedSound,
    fired: bool,
}

/// The group names cast to the player entity.
pub fn player_group(name: &str) -> bool {
    cast_target(name) == Some(BODY)
}

/// The player tree a cast group plays on: the body (0) or an arm (1 left,
/// 2 right); the cast mask bit of target `t` is `1 << t`.
pub fn cast_target(name: &str) -> Option<usize> {
    match name {
        "PlayerBody" | "PlayerCamera" => Some(BODY),
        "PlayerLeftArm" => Some(1),
        "PlayerRightArm" => Some(2),
        "PlayerLeftEquipment" => Some(3),
        "PlayerRightEquipment" => Some(4),
        _ => None,
    }
}

pub const BODY: usize = 0;
pub const TARGETS: usize = 5;

impl PreparedScene {
    /// The scene's player group (at most one per scene in the installed
    /// decls).
    pub fn player_group(&self, scene: usize) -> Result<Option<usize>, String> {
        self.group(scene, BODY)
    }

    /// The scene's group cast to tree `target`.
    pub fn group(&self, scene: usize, target: usize) -> Result<Option<usize>, String> {
        self.select_group(scene, target, None)
    }

    /// Apply the same authored model applicability as the shared SFX owner.
    /// Mutually exclusive model groups may share the same cast target.
    pub fn group_for_model(&self, scene: usize, target: usize, model: &str) -> Result<Option<usize>, String> {
        self.select_group(scene, target, Some(model))
    }

    /// The scene's groups cast to `target`, in scene order.
    fn groups_on<'a>(&'a self, scene: usize, target: usize, model: Option<&'a str>) -> impl Iterator<Item = usize> + 'a {
        self.decl.scenes[scene].groups.iter().enumerate()
            .filter(move |(_, g)| !model.is_some_and(|model| !g.specific_models.is_empty() && !g.specific_models.iter().any(|value| value == model)))
            .filter(move |(_, g)| self.decl.group_name(g.group).and_then(cast_target) == Some(target))
            .map(|(i, _)| i)
    }

    /// The group whose anim track drives `target`. 8542b0 casts PlayerBody
    /// and PlayerCamera to the same entity; each group's track write
    /// (1407e2ad0 -> 1406eb410) clears and rebuilds that entity's midnight
    /// node, so the last group in scene order determines the tree.
    fn select_group(&self, scene: usize, target: usize, model: Option<&str>) -> Result<Option<usize>, String> {
        Ok(self.groups_on(scene, target, model).last())
    }

    fn target_clips(&self, scene: usize, target: usize) -> &[usize] {
        match target {
            BODY => &self.clips[scene],
            1 | 2 => self.arm_clips[target - 1].get(scene).map_or(&[], Vec::as_slice),
            3 | 4 => self.equipment_clips[target - 3].get(scene).map_or(&[], Vec::as_slice),
            _ => &[],
        }
    }
}

#[derive(Debug)]
pub struct ActionSceneDiagnostic {
    pub path: Option<String>,
    pub handle: u32,
    pub cast_mask: u8,
    pub scene_uid: u32,
    pub tracks: [usize; TARGETS],
    pub aliases: [Vec<String>; TARGETS],
}

/// The animations of each player tree (body, left arm, right arm).
pub type Cast<'a> = [&'a [EdgeAnimation]; TARGETS];

/// One scheduled clip (0x98 record): scene ms → anim ms between two
/// segments.
#[derive(Clone, Debug)]
struct Clip {
    anim: usize,
    scene_from: i32,
    scene_to: i32,
    anim_from: i32,
    anim_to: i32,
    priority: i32,
    curve: Option<FloatCurve>,
    main: bool,
    sounds: Arc<[AnimationSound]>,
    effects:Arc<[crate::assets::animation_rig::animset_effects::Effect]>,
}

/// One clip of the frame's midnightData.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ClipInfo {
    anim: usize,
    weight: f32,
    frame: f32,
    main: bool,
}

struct Instance {
    handle: u32,
    scene: Arc<PreparedScene>,
    current: usize,
    time: i32,
    abs: i64,
    carry: f32,
    rate: f32,
    clips: [Vec<Clip>; TARGETS],
    events: Vec<(crate::entities::midnight::EventKey, usize)>,
    sounds: Vec<ScheduledSound>,
    playing_sounds: std::collections::HashMap<(u32, Emitter), bool>,
    equipment_emitters: [Option<Emitter>; 2],
    parts: u8,
    /// The cast trees' md6s (body, left arm, right arm) for CONDMD6.
    md6: [String; TARGETS],
    graph: GraphState,
}

/// 140652ed0's per-name random state (1407cd330): the output count, the
/// outputs already drawn (20 bits) and m_autoDisable / m_looping.
#[derive(Clone, Copy)]
struct RandomState { count: u32, used: u32, auto_disable: bool, looping: bool }

struct GraphState { booleans: Vec<bool>, ints: Vec<i32>, randoms: Vec<Option<RandomState>>, loops: Vec<i32>, loop_limits: Vec<i32> }
impl GraphState {
    fn new(scene: &PreparedScene) -> Self {
        use crate::entities::midnight::IntCondition;
        // 1407ce5f0 creates one variable or random state per name; lookups
        // by name (140195650) find the first int node carrying it.
        let first_int = |name: &str| scene.decl.int_nodes.iter().find(|node| node.name == name).unwrap();
        Self { booleans: scene.decl.bool_nodes.iter().map(|node|
            scene.decl.bool_nodes.iter().find(|first| first.name == node.name).unwrap().default).collect(),
            ints: vec![0; scene.decl.int_nodes.len()],
            randoms: scene.decl.int_nodes.iter().map(|node| match first_int(&node.name) {
                first @ crate::entities::midnight::CondInt { condition: IntCondition::Random { auto_disable, looping }, .. } =>
                    Some(RandomState { count: first.outputs.len() as u32, used: 0, auto_disable: *auto_disable, looping: *looping }),
                _ => None,
            }).collect(),
            loops: vec![-1; scene.decl.loops.len()], loop_limits: scene.decl.loops.iter().map(|node| node.count).collect() }
    }
    fn int_slot(scene: &PreparedScene, index: usize) -> usize {
        scene.decl.int_nodes.iter().position(|node| node.name == scene.decl.int_nodes[index].name).unwrap()
    }
    /// 140652ed0 with the game LCG (1408ac150): -1 when every output of an
    /// auto-disabling node has been drawn.
    fn draw(state: &mut RandomState, random: &mut u32) -> Result<i32, String> {
        let mut next = || { *random = random.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f); (*random >> 10) & 0x7fff };
        if !state.auto_disable {
            if state.count == 0 { return Err("Midnight random condition has no outputs".into()); }
            let value = next() % state.count;
            state.used |= (1 << (value & 0x1f)) & 0xfffff;
            return Ok(value as i32);
        }
        let open: Vec<u32> = (0..state.count).filter(|&index| state.used & (1 << (index & 0x1f)) & 0xfffff == 0).collect();
        if open.is_empty() { return Ok(-1); }
        let value = open[(next() as i32 % open.len() as i32) as usize];
        if state.looping && open.len() == 1 { state.used = 0; } else { state.used |= (1 << (value & 0x1f)) & 0xfffff; }
        Ok(value as i32)
    }
    fn loop_slot(scene: &PreparedScene, index: usize) -> usize {
        scene.decl.loops.iter().position(|node| node.name == scene.decl.loops[index].name).unwrap()
    }
}

/// One tree's midnight node: its anim track's data and snapshot blender.
#[derive(Default)]
struct Node {
    /// The anim track's midnightData of the latest write.
    data: Vec<ClipInfo>,
    /// The previous tree evaluation's inputs, for the root delta.
    previous: Vec<ClipInfo>,
    /// Snapshot blend: pose, elapsed, duration.
    snapshot: Option<(Pose, f32, f32)>,
    pending_snapshot: Option<f32>,
    last: Option<Pose>,
}

/// The player's midnight manager subset and the player trees' midnight
/// nodes (body, left arm, right arm).
#[derive(Default)]
pub struct Midnight {
    /// Most recent scene-start failure, retained for the F8 report.
    pub last_start_error: Option<String>,
    /// The playing scene instances (catfall plays its arm scenes beside the
    /// body scene).
    instances: Vec<Instance>,
    next_handle: u32,
    sound_requests: Vec<SoundRequest>,
    animation_sound_receipts: Vec<AnimationSoundReceipt>,
    animation_effect_receipts:Vec<AnimationEffectReceipt>,
    equipment_emitters: [Option<Emitter>; 2],
    point_events: Vec<PointEvent>,
    nodes: [Node; TARGETS],
    /// m_midnightWeight: the largest full-body clip weight.
    pub weight: f32,
    /// The frame's root motion: has full body, translation, yaw (degrees).
    pub root: Option<(Vec3, f32)>,
}

/// FUN_14018fe90 with KeyCurve_Float slot 13 (FUN_140182e00).
pub(crate) fn weight(curve: &Option<FloatCurve>, x: i32) -> f32 {
    let Some(c) = curve else { return 1.0 };
    let n = c.times.len();
    if n == 0 {
        return 1.0;
    }
    let stride = (c.values.len() / n).max(1);
    let v = if n == 1 || x < c.times[0] {
        c.values[0]
    } else if x >= c.times[n - 1] {
        c.values[stride * (n - 1)]
    } else {
        let i = (0..n - 1).find(|&i| c.times[i] < x && x <= c.times[i + 1]).unwrap_or(0);
        let span = (c.times[i + 1] - c.times[i]) as f32 * 0.001;
        let u = if span > f32::EPSILON { (x - c.times[i]) as f32 * 0.001 / span } else { (x - c.times[i]) as f32 * 0.001 };
        match c.mode {
            1 => c.values[i],
            2 => c.values[i] + (c.values[i + 1] - c.values[i]) * u,
            4 if stride == 3 => {
                let s = 1.0 - u;
                let (p0, p3) = (c.values[3 * i], c.values[3 * i + 3]);
                let (p1, p2) = (p0 + c.values[3 * i + 2], p3 + c.values[3 * i + 4]);
                s * s * s * p0 + 3.0 * u * s * s * p1 + 3.0 * u * u * s * p2 + u * u * u * p3
            }
            _ => c.values[stride * i],
        }
    };
    v.clamp(0.0, 1.0)
}

/// A point event retains the real receiving cast entity; equipment events
/// must reach the weapon component instead of the body state machine.
#[derive(Clone, Debug, PartialEq)]
pub struct PointEvent {
    pub name: String,
    pub handle: u32,
    pub target: usize,
    pub class: String,
    pub properties: Arc<[(String, crate::entities::Value)]>,
    /// None is a point; timed-event tracks deliver true on entry and false
    /// on exit, including an interruption (84e0c0).
    pub active: Option<bool>,
}

impl Midnight {
    pub(super) fn write_combat_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(report, "  current Midnight instances={} (first 8):", self.instances.len());
        for instance in self.instances.iter().take(8) {
            let _ = writeln!(report, "    handle={} node={} time_ms={} absolute_ms={} rate={} parts={:#x} clips={:?} queued_events={} scheduled_sounds={}", instance.handle, instance.current, instance.time, instance.abs, instance.rate, instance.parts, instance.clips.each_ref().map(Vec::len), instance.events.len(), instance.sounds.len());
            for (target, clips) in instance.clips.iter().enumerate() {
                for clip in clips.iter().take(2) {
                    let _ = writeln!(report, "      target={} animation={} scene_ms={}..{} animation_ms={}..{} priority={} main={}", target, clip.anim, clip.scene_from, clip.scene_to, clip.anim_from, clip.anim_to, clip.priority, clip.main);
                }
            }
        }
    }
    ///7d8fe0 resolves the live instance;7d1740 writes its playback rate.
    ///A finished or absent handle is a normal false result.
    pub fn set_rate(&mut self, handle: u32, rate: f32) -> Result<bool, String> {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.handle == handle) else { return Ok(false); };
        instance.rate = rate;
        Ok(true)
    }
    /// 7d9050 -> 7d1770 only updates an existing named instance variable.
    /// It neither adds undeclared variables nor advances the graph.
    pub fn set_bool(&mut self, handle: u32, name: &str, value: bool) -> bool {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.handle == handle) else { return false };
        let mut found = false;
        for (index, node) in instance.scene.decl.bool_nodes.iter().enumerate() {
            if node.name == name { instance.graph.booleans[index] = value; found = true; }
        }
        found
    }
    /// 7d7180 changes the existing named loop's iteration limit. Its current
    /// progress and authored break outputs belong to the same instance.
    pub fn set_loop_count(&mut self, handle: u32, name: &str, count: i32) -> bool {
        let Some(instance) = self.instances.iter_mut().find(|instance| instance.handle == handle) else { return false };
        let mut found = false;
        for (index, node) in instance.scene.decl.loops.iter().enumerate() {
            if node.name == name { instance.graph.loop_limits[index] = count; found = true; }
        }
        found
    }

    /// 7d4cb0 -> 7cb9e0: the last argument forces an active loop's immediate
    /// break; it is not a message payload. Deferred breaks take the authored
    /// end output on the next visit to the named graph loop.
    pub fn send_message(&mut self, handle: u32, name: &str, force: bool, cast: Cast, random: &mut u32) -> Result<bool, String> {
        let Some(i) = self.instances.iter().position(|instance| instance.handle == handle) else { return Ok(false) };
        let instance = &mut self.instances[i];
        let Some((index, node)) = instance.scene.decl.loops.iter().enumerate().find(|(_, node)| node.name == name) else { return Ok(false) };
        let active = instance.graph.loops[index] >= 0;
        instance.graph.loops[index] = if active { -2 } else { -3 };
        if !active || !(force || node.break_instantly) { return Ok(true); }
        let blend = node.break_blend;
        let prepared = instance.scene.clone();
        let parts = instance.parts;
        let md6 = instance.md6.clone();
        let next = Self::resolve_graph(&prepared, prepared.decl.scenes[instance.current].next,
            md6.each_ref().map(String::as_str), &mut instance.graph, random)?;
        let entered = next.map(|index| self.enter(&prepared, index, cast, parts, md6.each_ref().map(String::as_str))
            .map(|(clips, events)| (index, clips, events))).transpose()?;
        Self::exit_events(&self.instances[i], None, &mut self.point_events);
        Self::interrupt_sounds(&self.instances[i], &mut self.sound_requests);
        if let Some((index, clips, events)) = entered {
            let instance = &mut self.instances[i];
            (instance.current, instance.time, instance.abs, instance.carry) = (index, 0, 0, 0.0);
            (instance.clips, instance.events) = (clips, events);
            instance.playing_sounds.clear();
            instance.sounds = Self::schedule_sounds(&prepared, index, parts, instance.equipment_emitters);
            Self::initial_sounds(instance, &mut self.sound_requests);
            Self::initial_events(instance, &mut self.point_events);
        } else { self.instances.remove(i); }
        for (target, node) in self.nodes.iter_mut().enumerate() {
            if parts & (1 << target) != 0 { node.pending_snapshot = Some(blend); }
        }
        self.write(cast);
        Ok(true)
    }

    /// 8542b0 casts the actual inventory entity. A scene keeps that binding
    /// even when a later equipment change reuses the same hand slot.
    pub fn set_equipment_entity(&mut self, side: usize, entity: Option<crate::gameplay::entity_lifetime::EntityId>) {
        if let Some(emitter) = self.equipment_emitters.get_mut(side) {
            *emitter = entity.map(|entity| Emitter::Equipment { entity, part: 1 << (side + 3) });
        }
    }

    /// 8542b0 casts actual entity trees, not a reusable equipment slot's
    /// skeleton. Host slot replacement retires the outgoing entity's target
    /// before installing another rig; its snapshot and clip indices cannot
    /// leak into the replacement entity.
    pub fn retire_target(&mut self, target: usize) {
        let Some(node) = self.nodes.get_mut(target) else { return };
        *node = Node::default();
        let part = 1u8 << target;
        self.point_events.retain(|event| event.target != target);
        for instance in &mut self.instances {
            Self::exit_events(instance, Some(target), &mut self.point_events);
            for (&(event, emitter), &stop) in &instance.playing_sounds {
                if stop && emitter.player_part() & part != 0 { self.sound_requests.push(SoundRequest::Stop { event, emitter }); }
            }
            instance.playing_sounds.retain(|(_, emitter), _| emitter.player_part() & part == 0);
            instance.sounds.retain(|sound| sound.sound.emitter.player_part() & part == 0);
            instance.events.retain(|(_, owner)| *owner != target);
            instance.clips[target].clear();
            instance.parts &= !part;
        }
        self.instances.retain(|instance| instance.parts != 0);
    }

    pub fn take_point_events(&mut self) -> Vec<PointEvent> { std::mem::take(&mut self.point_events) }
    pub fn drain_point_events(&mut self) -> std::vec::Drain<'_, PointEvent> { self.point_events.drain(..) }

    pub fn take_sound_requests(&mut self) -> Vec<SoundRequest> {
        std::mem::take(&mut self.sound_requests)
    }

    fn schedule_sounds(prepared: &PreparedScene, index: usize, parts: u8, equipment_emitters: [Option<Emitter>; 2]) -> Vec<ScheduledSound> {
        prepared.sounds[index].iter().filter(|s| s.emitter.player_part() & parts != 0)
            .filter_map(|source| {
                let mut sound = source.clone();
                sound.emitter = match sound.emitter {
                    Emitter::PlayerLeftEquipment => equipment_emitters[0]?,
                    Emitter::PlayerRightEquipment => equipment_emitters[1]?,
                    emitter => emitter,
                };
                Some(ScheduledSound { sound, fired: false })
            }).collect()
    }

    /// Track initialization 1407dfbd0 selects point keys at zero and clips
    /// containing zero. Zero-duration SFX clips fire on the next time step.
    fn initial_events(inst: &Instance, out: &mut Vec<PointEvent>) {
        // 7dfbd0 selects forward point keys at the scene's initial time.
        out.extend(inst.events.iter().filter(|(event, _)| event.duration_ms.map_or(event.time_ms == inst.time,
            |duration| event.time_ms <= inst.time && inst.time < event.time_ms.saturating_add(duration))).map(|(event, target)|
            PointEvent { name: event.event.clone(), handle: inst.handle, target: *target,
                class: event.class.clone(), properties: event.properties.clone(), active: event.duration_ms.map(|_| true) }));
    }

    fn exit_events(inst: &Instance, target: Option<usize>, out: &mut Vec<PointEvent>) {
        out.extend(inst.events.iter().filter(|(event, owner)| target.is_none_or(|target| target == *owner)
            && event.duration_ms.is_some_and(|duration| event.time_ms <= inst.time
                && inst.time < event.time_ms.saturating_add(duration))).map(|(event, target)|
            PointEvent { name: event.event.clone(), handle: inst.handle, target: *target,
                class: event.class.clone(), properties: event.properties.clone(), active: Some(false) }));
    }

    fn initial_sounds(inst: &mut Instance, out: &mut Vec<SoundRequest>) {
        for sound in &mut inst.sounds {
            let s = &sound.sound;
            let selected = if s.key.is_clip {
                s.key.time_ms <= inst.time && s.key.time_ms.saturating_add(s.key.duration_ms).saturating_sub(1) >= inst.time
            } else { s.key.time_ms == inst.time };
            if selected {
                out.push(SoundRequest::Post { event: s.event, emitter: s.emitter });
                inst.playing_sounds.insert((s.event, s.emitter), s.key.stop_on_interruption);
                sound.fired = true;
            }
        }
    }

    /// SFX interruption cleanup 1408279c0 stops flagged events on their
    /// owning audio component. Other one-shot events continue playing.
    fn interrupt_sounds(inst: &Instance, out: &mut Vec<SoundRequest>) {
        for (&(event, emitter), &stop) in &inst.playing_sounds {
            if stop { out.push(SoundRequest::Stop { event, emitter }); }
        }
    }
    /// The main clip of the latest data (mainAnimPtr / mainAnimFrame):
    /// (clip, frame).
    pub fn main_clip(&self) -> Option<(usize, f32)> {
        self.nodes[BODY].data.iter().rev().find(|c| c.main).map(|c| (c.anim, c.frame))
    }

    /// The scene playing on the body tree.
    pub fn playing(&self) -> Option<u32> {
        self.instances.iter().find(|i| i.parts & 1 << BODY != 0).map(|i| i.handle)
    }

    /// Capture each scene segment once across an action; repeated frames do
    /// not allocate. This retains transitions even if F8 is pressed later.
    pub fn capture_action_scenes(&self, scenes: &std::collections::HashMap<String, Arc<PreparedScene>>,
        history: &mut Vec<ActionSceneDiagnostic>) {
        for instance in &self.instances {
            let scene_uid = instance.scene.decl.scenes[instance.current].uid;
            if history.iter().any(|entry| entry.handle == instance.handle && entry.scene_uid == scene_uid) { continue; }
            history.push(ActionSceneDiagnostic {
                path: scenes.iter().find_map(|(path, scene)| Arc::ptr_eq(scene, &instance.scene).then(|| path.clone())),
                handle: instance.handle,
                cast_mask: instance.parts,
                scene_uid,
                tracks: std::array::from_fn(|target| instance.clips[target].len()),
                aliases: std::array::from_fn(|target| {
                    instance.scene.group(instance.current, target).ok().flatten().map(|group|
                        instance.scene.decl.scenes[instance.current].groups[group].anim_keys.iter()
                            .map(|key| key.anim.clone()).collect()).unwrap_or_default()
                }),
            });
        }
    }

    /// FUN_1407cf250(inst, ignoreBlendOut = 1): the scene reached its end.
    pub fn finished(&self, handle: u32) -> bool {
        !self.instances.iter().any(|i| i.handle == handle)
    }

    /// FUN_1407cc650: resolve a graph element; CONDMD6 (FUN_1407ccdc0)
    /// reads the md6 of the entity its node's group is cast to (`md6`:
    /// body, left arm, right arm). `random` is the game LCG (1408ac150)
    /// random int conditions draw from.
    fn resolve_graph(scene: &PreparedScene, mut next: Next, md6: [&str; TARGETS], graph: &mut GraphState, random: &mut u32) -> Result<Option<usize>, String> {
        // The native walker has no fixed graph-depth limit. A host validation
        // rejects a repeated pure CONDMD6 node instead of truncating valid
        // declaration chains at an arbitrary number of elements.
        let mut visited = std::collections::HashSet::new();
        let mut path = Vec::new();
        loop {
            if !scene.decl.loops.is_empty() { path.push(next); }
            match next {
                Next::Final => return Ok(None),
                Next::Scene(uid) => {
                    // 195b50/7cc8e0/7cd1c0 associate loops with their
                    // looping output in the traversed path. A message before
                    // first entry takes the loop's end output instead; an
                    // entered loop is active already during its first scene.
                    if let Some((index, node)) = scene.decl.loops.iter().enumerate()
                        .find(|(index, node)| graph.loops[GraphState::loop_slot(scene, *index)] == -3 && path.contains(&node.looping)) {
                        graph.loops[GraphState::loop_slot(scene, index)] = -2;
                        next = Next::Node(node.uid);
                        path.clear();
                        visited.clear();
                        continue;
                    }
                    for (index, node) in scene.decl.loops.iter().enumerate() {
                        let slot = GraphState::loop_slot(scene, index);
                        if path.contains(&node.looping) && graph.loops[slot] < 0 { graph.loops[slot] = 0; }
                    }
                    return scene.decl.scenes.iter().position(|s| s.uid == uid).map(Some).ok_or_else(|| format!("missing midnight scene {uid}"));
                }
                Next::Node(uid) => {
                    if !visited.insert(uid) { return Err("midnight graph does not reach a scene".into()); }
                    if let Some((index, node)) = scene.decl.bool_nodes.iter().enumerate().find(|(_, node)| node.uid == uid) {
                        next = if graph.booleans[index] { node.when_true } else { node.when_false };
                        continue;
                    }
                    if let Some((index, node)) = scene.decl.int_nodes.iter().enumerate().find(|(_, node)| node.uid == uid) {
                        // 1407cccd0: output `value` below the output count,
                        // otherwise (and for -1) the default output.
                        use crate::entities::midnight::IntCondition;
                        let slot = GraphState::int_slot(scene, index);
                        let value = match node.condition {
                            IntCondition::Static => graph.ints[slot],
                            IntCondition::Kiscule => -1,
                            IntCondition::Random { .. } => GraphState::draw(
                                graph.randoms[slot].as_mut().ok_or("Midnight random condition has no state")?, random)?,
                        };
                        next = match usize::try_from(value) {
                            Ok(value) if value < node.outputs.len() => node.outputs[value],
                            _ if value == -1 || value >= node.outputs.len() as i32 => node.default,
                            _ => return Err(format!("Midnight int condition {} has no output {value}", node.name)),
                        };
                        continue;
                    }
                    if let Some((index, node)) = scene.decl.loops.iter().enumerate().find(|(_, node)| node.uid == uid) {
                        // 7cd040 /813280: break markers -3/-2 leave the
                        // loop; ordinary visits increment before count test.
                        let slot = GraphState::loop_slot(scene, index);
                        let limit = graph.loop_limits[slot];
                        let state = &mut graph.loops[slot];
                        let broken = matches!(*state, -3 | -2);
                        if !broken { *state = state.saturating_add(1); }
                        if broken || (limit >= 0 && *state >= limit) {
                            *state = -1;
                            next = node.end;
                        } else { next = node.looping; }
                        continue;
                    }
                    let node = scene.decl.nodes.iter().find(|n| n.uid == uid).ok_or_else(|| format!("missing midnight node {uid}"))?;
                    let target = cast_target(&node.group).ok_or_else(|| format!("CONDMD6 on group {} is not decoded", node.group))?;
                    let md6 = md6[target];
                    // The base-compatibility fallback (FUN_1401c8cc0) is not traced.
                    next = node.next.iter().find(|(m, _)| m == md6).map(|(_, n)| *n).ok_or_else(|| format!("no CONDMD6 match for {md6}"))?;
                }
            }
        }
    }

    /// The clip schedule of every cast tree and the body group's events.
    fn enter(&self, prepared: &Arc<PreparedScene>, index: usize, cast: Cast, parts: u8, md6: [&str; TARGETS]) -> Result<([Vec<Clip>; TARGETS], Vec<(crate::entities::midnight::EventKey, usize)>), String> {
        let mut schedules: [Vec<Clip>; TARGETS] = Default::default();
        let mut events = Vec::new();
        for (target, clips) in schedules.iter_mut().enumerate() {
            if parts & (1 << target) == 0 {
                continue;
            }
            let Some(g) = prepared.group_for_model(index, target, md6[target])? else { continue };
            let animations = cast[target];
            let group = &prepared.decl.scenes[index].groups[g];
            let prepared_clips = prepared.target_clips(index, target);
            if prepared_clips.len() != group.anim_keys.len() {
                return Err("midnight scene was not prepared for this player".into());
            }
            let mut next_priority = 0;
            for (k, key) in group.anim_keys.iter().enumerate() {
                if key.markers > 0 {
                    return Err("midnight clip markers are not decoded".into());
                }
                let anim = prepared_clips[k];
                let anim_ms = (animations[anim].duration() * 1000.0) as i32;
                let length = if key.stretched { key.duration_ms } else { anim_ms };
                // FUN_14018f250: the key priority over the clips already scheduled.
                let priority = key.priority + next_priority;
                next_priority = priority + 1;
                clips.push(Clip {
                    anim,
                    scene_from: key.start_ms,
                    scene_to: key.start_ms - 1 + length,
                    anim_from: key.start_offset,
                    anim_to: anim_ms - key.end_offset,
                    priority,
                    curve: key.curve.clone(),
                    main: key.main_clip,
                    sounds: prepared.animation_sounds.get(target, anim),
                    effects:prepared.animation_sounds.get_effects(target,anim),
                });
            }
            // Every group cast to this tree keeps its own event track.
            for g in prepared.groups_on(index, target, Some(md6[target])) {
                events.extend(prepared.decl.scenes[index].groups[g].events.iter().map(|event| (event.clone(), target)));
            }
        }
        Ok((schedules, events))
    }

    /// FUN_1408549c0 → FUN_1407d6040 / FUN_1407d9270: start at t = 0 (the
    /// first data is written now). An interrupted previous scene snapshots.
    pub fn start(&mut self, prepared: &Arc<PreparedScene>, md6: [&str; TARGETS], rate: f32, cast: Cast, random: &mut u32) -> Result<u32, String> {
        self.start_with_parts(prepared, md6, rate, cast, 1, random)
    }

    /// `parts` is the cast mask (1 body, 2 left arm, 4 right arm).
    pub fn start_with_parts(&mut self, prepared: &Arc<PreparedScene>, md6: [&str; TARGETS], rate: f32, cast: Cast, parts: u8, random: &mut u32) -> Result<u32, String> {
        self.start_with_parts_at_seconds(prepared, md6, rate, cast, parts, 0.0, random)
    }

    /// 86c6a0 -> 7d91c0 -> 7d2160: chained attacks start at the authored
    /// scene offset. Keys before that time are not replayed; points exactly
    /// at the offset and clips containing it initialize there (7dfbd0).
    pub fn start_with_parts_at_seconds(&mut self, prepared: &Arc<PreparedScene>, md6: [&str; TARGETS], rate: f32, cast: Cast, parts: u8, seconds: f32, random: &mut u32) -> Result<u32, String> {
        self.start_with_bools(prepared, md6, rate, cast, parts, seconds, &[], random)
    }

    /// 8549c0 invokes OnSceneStarted before Start resolves its first node.
    /// These named values therefore apply before the initial graph walk.
    pub fn start_with_bools(&mut self, prepared: &Arc<PreparedScene>, md6: [&str; TARGETS], rate: f32, cast: Cast, parts: u8, seconds: f32, booleans: &[(&str, bool)], random: &mut u32) -> Result<u32, String> {
        self.start_with_variables(prepared, md6, rate, cast, parts, seconds, booleans, &[], random)
    }
    /// Scene-start callbacks63a7c0 set Burst/BurstLoop before graph Start,
    /// just as Windblast initializes its bool before conditional resolution.
    pub fn start_with_variables(&mut self, prepared: &Arc<PreparedScene>, md6: [&str; TARGETS], rate: f32, cast: Cast,
        parts: u8, seconds: f32, booleans: &[(&str, bool)], loop_counts: &[(&str, i32)], random: &mut u32) -> Result<u32, String> {
        let mut graph = GraphState::new(prepared);
        for (index, node) in prepared.decl.bool_nodes.iter().enumerate() {
            if let Some((_, value)) = booleans.iter().rev().find(|(name, _)| *name == node.name) { graph.booleans[index] = *value; }
        }
        for (index, node) in prepared.decl.loops.iter().enumerate() {
            if let Some((_, count)) = loop_counts.iter().rev().find(|(name, _)| *name == node.name) { graph.loop_limits[index] = *count; }
        }
        let Some(index) = Self::resolve_graph(prepared, prepared.decl.first, md6, &mut graph, random)? else { return Ok(0) };
        let time = (seconds * 1000.0) as i32;
        if time > prepared.decl.scenes[index].length_ms { return Err("Midnight StartTime is superior to first scene length".into()); }
        let (clips, events) = self.enter(prepared, index, cast, parts, md6)?;
        // A scene on a tree another instance plays interrupts it there.
        let handles: Vec<u32> = self.instances.iter().filter(|i| i.parts & parts != 0).map(|i| i.handle).collect();
        for handle in handles {
            self.interrupt_parts(handle, parts);
        }
        self.next_handle = self.next_handle.wrapping_add(1).max(1);
        let sounds = Self::schedule_sounds(prepared, index, parts, self.equipment_emitters);
        let mut inst = Instance {
            handle: self.next_handle, scene: prepared.clone(), current: index, time, abs: 0, carry: 0.0, rate,
            clips, events, sounds, playing_sounds: Default::default(), parts, md6: md6.map(str::to_owned),
            equipment_emitters: self.equipment_emitters,
            graph,
        };
        Self::initial_sounds(&mut inst, &mut self.sound_requests);
        Self::initial_events(&inst, &mut self.point_events);
        self.instances.push(inst);
        self.write(cast);
        Ok(self.next_handle)
    }

    /// FUN_1407d9590 (flags 0x600): an interrupting stop snapshots the
    /// last pose over the scene's m_blendOutTime.
    pub fn stop(&mut self, handle: u32) {
        self.point_events.retain(|event| event.handle != handle);
        if let Some(i) = self.instances.iter().position(|i| i.handle == handle) {
            let inst = self.instances.remove(i);
            Self::exit_events(&inst, None, &mut self.point_events);
            Self::interrupt_sounds(&inst, &mut self.sound_requests);
            for (target, node) in self.nodes.iter_mut().enumerate() {
                if inst.parts & 1 << target != 0 {
                    node.data.clear();
                    node.pending_snapshot = Some(inst.scene.decl.blend_out_time);
                }
            }
        }
    }

    /// 7d2160/7d9270 replace the overlapping cast trees, not every tree
    /// belonging to their scene.86c6a0's left-arm response must leave the
    /// body's/right-arm sword scene running on those other trees.
    fn interrupt_parts(&mut self, handle: u32, parts: u8) {
        let Some(index) = self.instances.iter().position(|instance| instance.handle == handle) else { return };
        let instance = &mut self.instances[index];
        let removed = instance.parts & parts;
        self.point_events.retain(|event| event.handle != handle || removed & (1 << event.target) == 0);
        for target in 0..TARGETS {
            if removed & (1 << target) == 0 { continue; }
            Self::exit_events(instance, Some(target), &mut self.point_events);
            instance.clips[target].clear();
            self.nodes[target].data.clear();
            self.nodes[target].pending_snapshot = Some(instance.scene.decl.blend_out_time);
        }
        for (&(event, emitter), &stop) in &instance.playing_sounds {
            if stop && emitter.player_part() & removed != 0 {
                self.sound_requests.push(SoundRequest::Stop { event, emitter });
            }
        }
        instance.playing_sounds.retain(|(_, emitter), _| emitter.player_part() & removed == 0);
        instance.sounds.retain(|sound| sound.sound.emitter.player_part() & removed == 0);
        instance.events.retain(|(_, target)| removed & (1 << target) == 0);
        instance.parts &= !removed;
        if instance.parts == 0 { self.instances.remove(index); }
    }

    /// The manager update before the component stages: the time step, the
    /// events (returned as (event name, handle)) and the anim data.
    pub fn advance(&mut self, dt: f32, cast: Cast, random: &mut u32) -> Result<Vec<(String, u32)>, String> {
        let mut messages = Vec::new();
        let mut ended_instances = Vec::new();
        for inst in &mut self.instances {
            let length = inst.scene.decl.scenes[inst.current].length_ms;
            let end = length - 1;
            let total = dt * inst.rate + inst.carry;
            let mut ms = (total * 1000.0) as i32;
            let from = inst.time;
            let mut to = from + ms;
            let mut ended = false;
            if to >= end {
                ms = end - from;
                to = end;
                ended = true;
            }
            inst.carry = total - ms as f32 * 0.001;
            let first_step = inst.abs == 0;
            inst.abs += ms.unsigned_abs() as i64;
            inst.time = to;
            for (event, target) in &inst.events {
                let k = event.time_ms;
                let entered = (from <= k && k < to) || (k == to && ended);
                if entered && !(k == from && first_step) {
                    if event.duration_ms.is_none() { messages.push((event.event.clone(), inst.handle)); }
                    self.point_events.push(PointEvent { name: event.event.clone(), handle: inst.handle, target: *target,
                        class: event.class.clone(), properties: event.properties.clone(), active: event.duration_ms.map(|_| true) });
                }
                if let Some(duration) = event.duration_ms {
                    let end = k.saturating_add(duration);
                    if (from < end && end <= to) || (ended && k <= to && to < end) {
                        self.point_events.push(PointEvent { name: event.event.clone(), handle: inst.handle, target: *target,
                            class: event.class.clone(), properties: event.properties.clone(), active: Some(false) });
                    }
                }
            }
            // The SFX track uses the same forward key crossing as 1407dfdd0.
            // Its clip keys at zero are eligible on the first step, unlike points.
            for sound in &mut inst.sounds {
                let s = &sound.sound;
                let k = s.key.time_ms;
                let entered = (from <= k && k < to) || (k == to && ended);
                if !sound.fired && entered && (s.key.is_clip || !(k == 0 && first_step)) {
                    self.sound_requests.push(SoundRequest::Post { event: s.event, emitter: s.emitter });
                    inst.playing_sounds.insert((s.event, s.emitter), s.key.stop_on_interruption);
                    sound.fired = true;
                }
            }
            if ended {
                ended_instances.push(inst.handle);
            }
        }
        // FUN_1407d2800: the next element; FINAL clears the data with no
        // snapshot (the ended instance writes no further frame).
        for handle in ended_instances {
            let i = self.instances.iter().position(|inst| inst.handle == handle).unwrap();
            let inst = &mut self.instances[i];
            let (prepared, md6, parts) = (inst.scene.clone(), inst.md6.clone(), inst.parts);
            match Self::resolve_graph(&prepared, prepared.decl.scenes[inst.current].next, md6.each_ref().map(String::as_str), &mut inst.graph, random)? {
                None => {
                    self.instances.remove(i);
                }
                Some(index) => {
                    let (clips, events) = self.enter(&prepared, index, cast, parts, md6.each_ref().map(String::as_str))?;
                    let inst = &mut self.instances[i];
                    (inst.current, inst.time, inst.clips, inst.events) = (index, 0, clips, events);
                    inst.abs = 0;
                    inst.sounds = Self::schedule_sounds(&prepared, index, inst.parts, inst.equipment_emitters);
                    Self::initial_sounds(inst, &mut self.sound_requests);
                    Self::initial_events(inst, &mut self.point_events);
                }
            }
        }
        self.write(cast);
        animation_sounds::cross(&self.instances, &self.nodes, &mut self.animation_sound_receipts,&mut self.animation_effect_receipts);
        Ok(messages)
    }

    /// FUN_1407e2ad0: the clips covering the scene time with their weights,
    /// ascending by priority.
    fn write(&mut self, cast: Cast) {
        for node in &mut self.nodes {
            node.data.clear();
        }
        for inst in &self.instances {
            let t = inst.time;
            for (target, node) in self.nodes.iter_mut().enumerate() {
                let animations = cast[target];
                let mut clips: Vec<&Clip> = inst.clips[target].iter().filter(|c| c.scene_from <= t && t <= c.scene_to).collect();
                clips.sort_by_key(|c| c.priority);
                for c in clips {
                    let frac = if c.scene_to == c.scene_from { 0.0 } else { (t - c.scene_from) as f32 / (c.scene_to - c.scene_from) as f32 };
                    let fps = animations[c.anim].frame_rate.ceil();
                    let frame = fps * (c.anim_from as f32 * 0.001 + (c.anim_to - c.anim_from) as f32 * frac * 0.001);
                    node.data.push(ClipInfo { anim: c.anim, weight: weight(&c.curve, t - c.scene_from), frame, main: c.main });
                }
            }
        }
    }

    /// Whether the playing scene writes clips to tree `target` this frame.
    pub fn drives(&self, target: usize) -> bool {
        !self.nodes[target].data.is_empty()
    }

    /// The tree update of the midnight node: root motion (FUN_1406ffde0)
    /// and the snapshot timer. `dt` is the tree delta.
    pub fn update(&mut self, dt: f32, animations: &[EdgeAnimation], rest: &[JointRestPose]) -> Result<(), String> {
        let full = |c: &ClipInfo| animations[c.anim].flags & 1 == 0 && animations[c.anim].properties & 0xff == 0;
        let node = &self.nodes[BODY];
        self.weight = node.data.iter().filter(|c| full(c)).map(|c| c.weight).fold(0.0, f32::max);
        let mut root = None;
        let (mut delta, mut yaw) = (Vec3::ZERO, 0.0f32);
        for c in &node.data {
            if !(c.weight > 0.0 && full(c)) {
                continue;
            }
            let animation = &animations[c.anim];
            if animation.flags & 0x1e == 0 {
                continue;
            }
            root = Some((Vec3::ZERO, 0.0));
            if let Some(p) = node.previous.iter().find(|p| p.anim == c.anim) {
                if c.frame > p.frame {
                    let joint = |frame: f32| -> Result<JointRestPose, String> {
                        Ok(animation.sample(rest, frame / animation.frame_rate)?.swap_remove(0))
                    };
                    let (m0, m1) = (joint(p.frame)?, joint(c.frame)?);
                    let r0 = Mat3::from_quat(Quat::from_array(m0.rotation));
                    let r1 = Mat3::from_quat(Quat::from_array(m1.rotation));
                    let rel = r0.transpose() * r1;
                    let d_yaw = rel.x_axis.y.atan2(rel.x_axis.x) * 57.29578;
                    let d_t = r0.transpose() * (Vec3::from_array(m1.translation) - Vec3::from_array(m0.translation));
                    let w = c.weight;
                    yaw = (1.0 - w) * yaw + w * d_yaw;
                    delta = (1.0 - w) * delta + w * d_t;
                }
            }
        }
        self.root = root.map(|_| (delta, yaw));
        self.update_node(BODY, dt);
        Ok(())
    }

    /// A tree's midnight node timers; the arm trees have no root motion
    /// (their entities are bound to the body).
    pub fn update_node(&mut self, target: usize, dt: f32) {
        let node = &mut self.nodes[target];
        node.previous = node.data.clone();
        if let Some((_, elapsed, duration)) = &mut node.snapshot {
            *elapsed += dt;
            if *elapsed >= *duration {
                node.snapshot = None;
            }
        }
    }

    /// The midnight node's pose over `loco` (blendarray leafMidnight_06..00,
    /// BLEND or ADD_RIGHT), then the snapshot blender. `last` records the
    /// output for a later snapshot.
    pub fn evaluate(&mut self, loco: Pose, animations: &[EdgeAnimation], rest: &[JointRestPose]) -> Result<Pose, String> {
        self.evaluate_node(BODY, loco, animations, rest)
    }

    /// Tree `target`'s midnight node over its input pose, then its snapshot
    /// blender.
    pub fn evaluate_node(&mut self, target: usize, loco: Pose, animations: &[EdgeAnimation], rest: &[JointRestPose]) -> Result<Pose, String> {
        let node = &mut self.nodes[target];
        let mut pose = loco;
        for c in &node.data {
            if c.weight <= 0.0 {
                continue;
            }
            let animation = &animations[c.anim];
            let last = (animation.frame_count as f32 - 1.0).max(0.0);
            let layer = sample(animation, rest, c.frame.min(last) / animation.frame_rate)?;
            if animation.flags & 1 != 0 {
                blend_add(&mut pose, &layer, c.weight);
            } else if c.weight >= 1.0 {
                pose = layer;
            } else {
                blend_lerp(&mut pose, &layer, c.weight);
            }
        }
        if let Some(duration) = node.pending_snapshot.take() {
            if let Some(last) = node.last.clone() {
                node.snapshot = Some((last, 0.0, duration));
            }
        }
        if let Some((snapshot, elapsed, duration)) = &node.snapshot {
            let factor = if *duration > 0.0 { (1.0 - elapsed / duration).max(0.0) } else { 0.0 };
            if factor > 0.0 {
                blend_lerp(&mut pose, snapshot, factor);
            }
        }
        node.last = Some(pose.clone());
        Ok(pose)
    }
}
