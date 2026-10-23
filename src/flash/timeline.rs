//! `flash.display.MovieClip` and sprite timeline execution.
//!
//! Frame advance follows `FUN_141027850`, placement `FUN_141023360` /
//! `FUN_1410260a0` / `FUN_1410262b0`, and gotoFrame `FUN_14107dda0`.
use super::avm2::builtins::{argument as arg, define, native_class, Spec};
use super::avm2::name::Name;
use super::avm2::object::{Class, NativeClass, NativeFn, Obj};
use super::avm2::value::Value;
use super::avm2::{Error, Result, Vm};
use super::display::{alloc, child_at_depth, depth_index, detach, dispatch_added, display, display_ref, link, DisplayKind, TimelineState};
use super::geom::{Affine, Colour};
use super::player::{self, CharRef};
use crate::assets::iggy::timeline::{kind, place, ColourTransform, Command, Place, BLEND_UNCHANGED, FILTERS_UNCHANGED};
use std::rc::Rc;

fn affine(m: &[f32; 6]) -> Affine {
    // Stored order: x' = m0·x + m1·y + m4, y' = m2·x + m3·y + m5.
    Affine { a: m[0] as f64, c: m[1] as f64, b: m[2] as f64, d: m[3] as f64, tx: m[4] as f64, ty: m[5] as f64 }
}

fn colour(c: &ColourTransform) -> Colour {
    Colour {
        mul: c.mul.map(|v| v as f64),
        add: c.add.map(|v| v as f64),
    }
}

/// `FUN_1410262b0`: apply a placement's properties to an instance.
fn apply_place(o: &Obj, p: &Place, reset: bool) {
    let Some(mut d) = display(o) else { return };
    if reset {
        // param_4 == 1: matrix, cxform, blend and filters reset first.
        d.set_matrix(Affine::default());
        d.colour = Colour::default();
        d.blend = 1;
        d.filters.clear();
    }
    if let Some(c) = &p.colour {
        d.colour = colour(c);
    }
    if p.blend != BLEND_UNCHANGED {
        d.blend = p.blend;
    }
    if p.has(place::CACHE_AS_BITMAP) {
        d.cache_as_bitmap = true;
    }
    if p.filter_count != FILTERS_UNCHANGED {
        d.timeline_filters = p.filters.clone();
    }
    if p.has(place::MATRIX) && !d.script_moved {
        d.set_matrix(affine(&p.matrix));
    }
    d.ratio = p.ratio as f64 / 65535.0;
    if p.has(place::CLIP_DEPTH) {
        d.clip_depth = p.clip_depth;
    }
    if let Some(n) = &p.name {
        d.name = n.as_str().into();
    }
}

/// The AS3 class bound to a character by its library's SymbolClass table.
fn bound_class(vm: &mut Vm, c: CharRef) -> Result<Option<Rc<Class>>> {
    let name = {
        let shared = player::state(vm);
        let p = shared.borrow();
        p.libraries[c.library].symbols.iter().find(|(_, &id)| id == c.id as u32).map(|(n, _)| n.clone())
    };
    match name {
        Some(n) => Ok(vm.class_by_name(&n)?.and_then(|v| v.as_object().and_then(Obj::as_class))),
        None => Ok(None),
    }
}

/// Allocate the display object for a placement without running its
/// constructor (`FUN_1410209c0` / `FUN_141025e50`): construction is pending.
fn instantiate(vm: &mut Vm, library: usize, p: &Place) -> Result<Option<Obj>> {
    // Placement by class name resolves the character through the SymbolClass
    // tables of all libraries (`FUN_141023b00`), then places it.
    let by_class = match &p.class_name {
        Some(name) => player::state(vm).borrow().character_for_class(name),
        None => None,
    };
    let (class, character) = if let (Some(class_name), None) = (&p.class_name, by_class) {
        let Some(class) = vm.class_by_name(class_name)?.and_then(|v| v.as_object().and_then(Obj::as_class)) else {
            return Err(Error::Internal(format!("Timeline tried to create object of unknown class {class_name}")));
        };
        (class, None)
    } else {
        let c = match (by_class, p.character) {
            (Some(c), _) => c,
            (None, Some(id)) => CharRef { library, id },
            (None, None) => return Ok(None),
        };
        let (library, id) = (c.library, c.id);
        let file = player::state(vm).borrow().file(library);
        // A bitmap character's linkage class is its BitmapData; the
        // placed display object is a Bitmap.
        let bound = match file.character_kind(id as usize).ok().flatten() {
            Some(kind::BITMAP) => None,
            _ => bound_class(vm, c)?,
        };
        match bound {
            Some(class) => (class, None),
            None => {
                let file = player::state(vm).borrow().file(library);
                let (package, name) = match file.character_kind(id as usize).ok().flatten() {
                    Some(kind::SPRITE) => ("flash.display", "MovieClip"),
                    Some(kind::SHAPE) => ("flash.display", "Shape"),
                    Some(kind::BITMAP) => ("flash.display", "Bitmap"),
                    Some(kind::EDIT_TEXT) => ("flash.text", "TextField"),
                    _ => return Ok(None),
                };
                (native_class(vm, package, name), Some(c))
            }
        }
    };
    let object = vm.allocate(&class)?;
    {
        let Some(mut d) = display(&object) else {
            return Err(Error::Internal(format!("{} is not a display object", class.name.qualified())));
        };
        d.constructed = false;
        if let Some(c) = character {
            d.character = Some(c);
        }
    }
    if let Some(c) = character {
        let file = player::state(vm).borrow().file(c.library);
        match file.character_kind(c.id as usize).ok().flatten() {
            Some(kind::SPRITE) => {
                let t = player::state(vm).borrow().timeline(c);
                if let Some(t) = t {
                    let mut d = display(&object).unwrap();
                    d.scale9 = t.scale9;
                    d.timeline = Some(TimelineState { timeline: t, source: c, current: 0, playing: true, scripts: Vec::new(), script_pending: false });
                }
            }
            Some(kind::EDIT_TEXT) => super::text_record::init_text(vm, &object, &file, c.id as usize)?,
            Some(kind::BITMAP) => super::text_record::init_bitmap(vm, &object, c)?,
            _ => {}
        }
    }
    Ok(Some(object))
}

/// `FUN_1410201c0`: assign a named timeline child to the parent's property.
fn bind_name(vm: &mut Vm, parent: &Obj, child: &Obj, name: &str) -> Result<()> {
    let target = Value::Object(parent.clone());
    let n = Name::public(name);
    let declared = parent.borrow().traits.as_ref().is_some_and(|t| t.find(name, &n.set).is_some());
    let dynamic = parent.class().is_none_or(|c| !c.sealed);
    if declared || dynamic {
        vm.set_property(&target, &n, Value::Object(child.clone()))?;
    }
    Ok(())
}

/// One PLACE command (`FUN_141023360`). New instances are linked, their own
/// first frame is placed and they are bound to the parent's named property;
/// constructors run later in `construct_pending`.
fn place_command(vm: &mut Vm, parent: &Obj, library: usize, p: &Place, frame: u32, reset: bool) -> Result<()> {
    let depth = p.depth as i32;
    let existing = child_at_depth(parent, depth);
    if p.creates() && !(reset && existing.is_some()) {
        if let Some(old) = &existing {
            detach(vm, old)?;
        }
        let Some(child) = instantiate(vm, library, p)? else { return Ok(()) };
        {
            let mut d = display(&child).unwrap();
            d.depth = Some(depth);
            d.placed_frame = frame;
        }
        apply_place(&child, p, false);
        let index = depth_index(parent, depth);
        link(parent, &child, index);
        // `FUN_141020830`: the child's own first frame, then the name binding.
        run_frame(vm, &child, 0)?;
        if let Some(name) = &p.name {
            bind_name(vm, parent, &child, name)?;
        }
        return Ok(());
    }
    if let Some(child) = existing {
        apply_place(&child, p, reset);
    }
    Ok(())
}

/// `FUN_141022100` / `FUN_1410237d0`: run pending constructors post-order
/// (children in depth order first), each followed by added/addedToStage.
pub fn construct_pending(vm: &mut Vm, o: &Obj) -> Result<()> {
    for child in super::display::children(o) {
        construct_pending(vm, &child)?;
        let pending = display_ref(&child).is_some_and(|d| !d.constructed);
        if pending {
            display(&child).unwrap().constructed = true;
            let class = child.class().unwrap();
            vm.run_constructor(&class, &Value::Object(child.clone()), &[])?;
            let added = dispatch_added(vm, &child);
            super::report(vm, added)?;
        }
    }
    Ok(())
}

fn timeline(o: &Obj) -> Option<TimelineState> {
    display_ref(o).and_then(|d| d.timeline.clone())
}

/// Run frame `frame` (0-based) of `o`'s timeline forwards (`FUN_141027850`).
fn run_frame(vm: &mut Vm, o: &Obj, frame: u32) -> Result<()> {
    let Some(t) = timeline(o) else { return Ok(()) };
    let frames = t.timeline.frames();
    let Some(commands) = frames.get(frame as usize) else { return Ok(()) };
    let commands: Vec<Command> = commands.to_vec();
    for c in &commands {
        match c {
            Command::Place(p) => place_command(vm, o, t.source.library, p, frame, false)?,
            Command::Remove { depth } => {
                if let Some(child) = child_at_depth(o, *depth as i32) {
                    detach(vm, &child)?;
                }
            }
            Command::TabIndex { depth, index, .. } => {
                if let Some(child) = child_at_depth(o, *depth as i32) {
                    display(&child).unwrap().tab_index = *index as i32;
                }
            }
            _ => {}
        }
    }
    if let Some(mut d) = display(o) {
        if let Some(t) = d.timeline.as_mut() {
            t.current = frame + 1;
            t.script_pending = true;
        }
    }
    Ok(())
}

/// The native Sprite/MovieClip constructor (`FUN_1410176b0`). The root
/// builds frame 1 here and constructs its subtree inside `super()`
/// (`FUN_1410240c0`); timeline children were placed before their
/// constructors ran. Script-created clips place their first frame here.
pub fn construct_frame(vm: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    let Some(o) = this.as_object() else { return Ok(Value::Undefined) };
    let unplaced = timeline(o).is_some_and(|t| t.current == 0);
    if unplaced {
        run_frame(vm, o, 0)?;
        construct_pending(vm, o)?;
    }
    Ok(Value::Undefined)
}

/// gotoFrame (`FUN_14107dda0`): `target` is 0-based. Newly placed children
/// are constructed (`FUN_1410237d0`) and the frame script becomes due.
pub fn goto(vm: &mut Vm, o: &Obj, target: u32) -> Result<()> {
    let Some(t) = timeline(o) else { return Ok(()) };
    let count = t.timeline.frame_count as u32;
    let target = target.min(count.saturating_sub(1));
    let current = t.current.saturating_sub(1);
    if t.current != 0 && target == current {
        return Ok(());
    }
    let frames = t.timeline.frames();
    if t.current == 0 || target < current {
        // Going backwards removes instances created after the target and
        // replays the list from the head.
        for child in super::display::children(o) {
            let created = display_ref(&child).and_then(|d| d.depth.map(|_| d.placed_frame));
            if created.is_some_and(|f| f > target) {
                detach(vm, &child)?;
            }
        }
        let old = current;
        for f in 0..=target {
            for c in frames.get(f as usize).copied().unwrap_or_default() {
                if let Command::Place(p) = c {
                    if p.end_frame as u32 > target {
                        let reset = target < old && old < p.end_frame as u32;
                        place_command(vm, o, t.source.library, p, f, reset)?;
                    }
                }
            }
        }
    } else {
        for f in current + 1..=target {
            for c in frames.get(f as usize).copied().unwrap_or_default() {
                match c {
                    Command::Place(p) if p.end_frame as u32 > target => place_command(vm, o, t.source.library, p, f, false)?,
                    Command::Remove { depth } => {
                        if let Some(child) = child_at_depth(o, *depth as i32) {
                            detach(vm, &child)?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(mut d) = display(o) {
        if let Some(tl) = d.timeline.as_mut() {
            tl.current = target + 1;
            tl.script_pending = true;
        }
    }
    construct_pending(vm, o)
}

/// Advance a playing clip one frame (`FUN_141022750` / `FUN_141027850`);
/// clips at the end loop to frame 0 unless they have one frame. New
/// children are placed with constructors pending.
pub fn advance(vm: &mut Vm, o: &Obj) -> Result<()> {
    let Some(t) = timeline(o) else { return Ok(()) };
    let count = t.timeline.frame_count as u32;
    if !t.playing || count <= 1 {
        return Ok(());
    }
    if t.current >= count {
        goto(vm, o, 0)
    } else {
        run_frame(vm, o, t.current)
    }
}

/// Advance every playing clip under `o`, post-order.
pub fn advance_tree(vm: &mut Vm, o: &Obj) -> Result<()> {
    for child in super::display::children(o) {
        advance_tree(vm, &child)?;
    }
    advance(vm, o)
}

/// Run the due frame script of `o` (`FUN_1410247a0`).
pub fn run_frame_script(vm: &mut Vm, o: &Obj) -> Result<()> {
    let script = {
        let Some(mut d) = display(o) else { return Ok(()) };
        let Some(t) = d.timeline.as_mut() else { return Ok(()) };
        if !t.script_pending {
            return Ok(());
        }
        t.script_pending = false;
        let frame = t.current.saturating_sub(1);
        t.scripts.iter().rev().find(|(f, _)| *f == frame).map(|(_, s)| s.clone())
    };
    if let Some(f) = script {
        let result = vm.call(&f, Value::Object(o.clone()), &[]).map(|_| ());
        super::report(vm, result)?;
    }
    Ok(())
}

/// `FUN_141024890`: frame scripts pre-order over the display tree.
pub fn run_frame_scripts(vm: &mut Vm, o: &Obj) -> Result<()> {
    run_frame_script(vm, o)?;
    for child in super::display::children(o) {
        run_frame_scripts(vm, &child)?;
    }
    Ok(())
}

/// `FUN_141023850`: apply queued gotos. Returns whether any ran.
pub fn run_deferred_gotos(vm: &mut Vm) -> Result<bool> {
    let mut ran = false;
    loop {
        let next = {
            let shared = player::state(vm);
            let mut p = shared.borrow_mut();
            if p.gotos.is_empty() {
                None
            } else {
                Some(p.gotos.remove(0))
            }
        };
        let Some((o, frame, play)) = next else { return Ok(ran) };
        set_playing(&o, play);
        goto(vm, &o, frame)?;
        ran = true;
    }
}

fn this_clip(vm: &mut Vm, this: &Value) -> Result<Obj> {
    match this.as_object() {
        Some(o) if display_ref(o).is_some() => Ok(o.clone()),
        _ => Err(vm.type_error(1034, "receiver is not a MovieClip.")),
    }
}

/// Resolve a frame argument (1-based number or label) to a 0-based frame.
/// An unknown label is no goto: `FUN_14107dd20` only records that the
/// label was not found and `FUN_14101cc40` returns without an error.
fn frame_arg(vm: &mut Vm, o: &Obj, v: &Value) -> Result<Option<u32>> {
    let t = timeline(o);
    match v {
        Value::String(s) => match s.parse::<u32>() {
            Ok(n) => Ok(Some(n.saturating_sub(1))),
            Err(_) => Ok(t.and_then(|t| t.timeline.label_frame(s))),
        },
        other => {
            let n = vm.to_number(other)?;
            Ok(Some((n.max(1.0) as u32) - 1))
        }
    }
}

/// `FUN_14107dbc0`: gotos are always deferred (`DAT_143eb3288` has no
/// writer) and consumed by `run_deferred_gotos`.
fn queue_goto(vm: &mut Vm, o: &Obj, frame: u32, play: bool) {
    player::state(vm).borrow_mut().gotos.push((o.clone(), frame, play));
}

fn set_playing(o: &Obj, playing: bool) {
    if let Some(mut d) = display(o) {
        if let Some(t) = d.timeline.as_mut() {
            t.playing = playing;
        }
    }
}

fn methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("play", |vm, this, _| {
            set_playing(&this_clip(vm, this)?, true);
            Ok(Value::Undefined)
        }),
        ("stop", |vm, this, _| {
            set_playing(&this_clip(vm, this)?, false);
            Ok(Value::Undefined)
        }),
        ("gotoAndPlay", |vm, this, a| {
            let o = this_clip(vm, this)?;
            if let Some(f) = frame_arg(vm, &o, &arg(a, 0))? {
                queue_goto(vm, &o, f, true);
            }
            Ok(Value::Undefined)
        }),
        ("gotoAndStop", |vm, this, a| {
            let o = this_clip(vm, this)?;
            if let Some(f) = frame_arg(vm, &o, &arg(a, 0))? {
                queue_goto(vm, &o, f, false);
            }
            Ok(Value::Undefined)
        }),
        ("nextFrame", |vm, this, _| {
            let o = this_clip(vm, this)?;
            let cur = timeline(&o).map_or(0, |t| t.current);
            queue_goto(vm, &o, cur, false);
            Ok(Value::Undefined)
        }),
        ("prevFrame", |vm, this, _| {
            let o = this_clip(vm, this)?;
            let cur = timeline(&o).map_or(1, |t| t.current);
            queue_goto(vm, &o, cur.saturating_sub(2), false);
            Ok(Value::Undefined)
        }),
        ("addFrameScript", |vm, this, a| {
            let o = this_clip(vm, this)?;
            let mut pairs = Vec::new();
            for pair in a.chunks(2) {
                let frame = vm.to_uint(&pair[0])?;
                pairs.push((frame, pair.get(1).cloned().unwrap_or_default()));
            }
            if let Some(mut d) = display(&o) {
                if let Some(t) = d.timeline.as_mut() {
                    for (frame, f) in pairs {
                        t.scripts.retain(|(x, _)| *x != frame);
                        if !f.is_nullish() {
                            if t.current == frame + 1 {
                                t.script_pending = true;
                            }
                            t.scripts.push((frame, f));
                        }
                    }
                }
            }
            Ok(Value::Undefined)
        }),
    ]
}

fn getters() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("currentFrame", |vm, this, _| Ok(timeline(&this_clip(vm, this)?).map_or(1, |t| t.current.max(1)).into())),
        ("totalFrames", |vm, this, _| Ok(timeline(&this_clip(vm, this)?).map_or(1, |t| t.timeline.frame_count as u32).into())),
        ("framesLoaded", |vm, this, _| Ok(timeline(&this_clip(vm, this)?).map_or(1, |t| t.timeline.frame_count as u32).into())),
        ("enabled", |_, _, _| Ok(true.into())),
        ("currentLabel", |vm, this, _| {
            let Some(t) = timeline(&this_clip(vm, this)?) else { return Ok(Value::Null) };
            let frame = t.current.saturating_sub(1);
            Ok(t.timeline.labels().into_iter().filter(|(_, f)| *f <= frame).last().map(|(n, _)| Value::from(n)).unwrap_or(Value::Null))
        }),
        ("currentLabels", |vm, this, _| {
            let labels = timeline(&this_clip(vm, this)?).map(|t| t.timeline.labels()).unwrap_or_default();
            let mut items = Vec::new();
            for (n, f) in labels {
                items.push(frame_label(vm, n, f + 1)?);
            }
            Ok(Value::Object(vm.array(items)))
        }),
        ("currentScene", |vm, this, _| scene(vm, this)),
        ("scenes", |vm, this, _| {
            let s = scene(vm, this)?;
            Ok(Value::Object(vm.array(vec![s])))
        }),
    ]
}

/// Every installed movie has a single scene ("Scene 1").
fn scene(vm: &mut Vm, this: &Value) -> Result<Value> {
    let o = this_clip(vm, this)?;
    let (frames, labels) = timeline(&o).map_or((1, Vec::new()), |t| (t.timeline.frame_count as u32, t.timeline.labels()));
    let mut items = Vec::new();
    for (n, f) in labels {
        items.push(frame_label(vm, n, f + 1)?);
    }
    let labels = Value::Object(vm.array(items));
    let class = native_class(vm, "flash.display", "Scene");
    let scene = Value::Object(vm.allocate(&class)?);
    vm.set_property(&scene, &Name::public("name"), "Scene 1".into())?;
    vm.set_property(&scene, &Name::public("labels"), labels)?;
    vm.set_property(&scene, &Name::public("numFrames"), frames.into())?;
    Ok(scene)
}

/// FrameLabel objects are allocated by natives (their AS3 constructor throws).
fn frame_label(vm: &mut Vm, name: String, frame: u32) -> Result<Value> {
    let class = native_class(vm, "flash.display", "FrameLabel");
    let label = Value::Object(vm.allocate(&class)?);
    vm.set_property(&label, &Name::public("name"), Value::from(name))?;
    vm.set_property(&label, &Name::public("frame"), Value::from(frame))?;
    Ok(label)
}

pub fn install(vm: &mut Vm, sprite: &Rc<Class>) {
    let object = vm.b().object.clone();
    let mut s = Spec::new("flash.display", "FrameLabel", Some(object.clone()));
    s.vars = vec![("name", Value::Null), ("frame", 0.into())];
    define(vm, s);
    let mut s = Spec::new("flash.display", "Scene", Some(object));
    s.vars = vec![("name", Value::Null), ("labels", Value::Null), ("numFrames", 0.into())];
    define(vm, s);

    let mut s = Spec::new("flash.display", "MovieClip", Some(sprite.clone()));
    s.sealed = false;
    s.native = NativeClass { allocate: Some(alloc(DisplayKind::MovieClip)), call: None, init: Some(construct_frame) };
    s.methods = methods();
    s.getters = getters();
    s.setters = vec![("enabled", |_, _, _| Ok(Value::Undefined))];
    define(vm, s);
}
