//! Mouse input (`artifacts/ui-pause/notes-mouse.md`): target selection
//! (`FUN_141021290`), moves with over/out/roll events (`FUN_141026f40`,
//! `FUN_141020dd0`), buttons with focus and click/doubleClick
//! (`FUN_141026b20`) and the wheel (`FUN_141027260`). Positions are stage
//! pixels; the host converts window pixels (`FUN_140033530`).
use super::avm2::object::Obj;
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::display::{children, display_ref, DisplayKind};
use super::geom::Affine;
use super::three_d::{self, Mat4};
use super::{events, player};
use crate::assets::iggy::shape::{Segment, Shape};

/// Double-click window (f32 at 0x142422dc0).
const DOUBLE_CLICK_SECONDS: f64 = 0.5;

/// Player mouse state (`P+0x29c4..0x2d78`).
#[derive(Clone, Default)]
pub struct MouseState {
    /// Stage position (`P+0x29d8/0x29dc`).
    pub position: (f64, f64),
    /// Left button down (`P+0x29c4`).
    pub button_down: bool,
    /// 0 none, 1 left, 2 right, 3 middle (`P+0x29c5`).
    pub press: u8,
    /// Double-click state machine (`P+0x29c6`) and reference time.
    pub double_state: u8,
    pub double_time: f64,
    pub double_target: Option<Obj>,
    /// Pressed object (`P+0x29f0`).
    pub pressed: Option<Obj>,
    /// Current over object (`P+0x2a00`, None = Stage) and innermost hit.
    pub over: Option<Obj>,
    pub inner: Option<Obj>,
    /// Ancestor chain of the over object (`P+0x2a58`).
    pub chain: Vec<Obj>,
}

/// A display object's mapping to stage pixels: 2D, or a 3D world.
#[derive(Clone, Copy)]
enum Xform {
    Flat(Affine),
    Deep(Mat4),
}

struct Picker<'a> {
    vm: &'a Vm,
    point: (f64, f64),
    projection: three_d::Projection,
    stage: Option<Obj>,
    root: Option<Obj>,
    script_masks: Vec<Obj>,
    over: Option<Obj>,
    inner: Option<Obj>,
}

fn kind(o: &Obj) -> Option<DisplayKind> {
    display_ref(o).map(|d| d.kind)
}

impl Picker<'_> {
    /// The point in `o`'s local space.
    fn local(&self, x: &Xform) -> Option<(f64, f64)> {
        let (px, py) = self.point;
        match x {
            Xform::Flat(m) => {
                let det = m.a * m.d - m.b * m.c;
                (det != 0.0).then(|| m.invert().apply(px, py))
            }
            // The inverse homography of `FUN_141027d80` on the projected plane.
            Xform::Deep(w) => {
                let f = self.projection.focal as f64;
                let [cx, cy] = self.projection.centre.map(|v| v as f64);
                let w: [f64; 16] = std::array::from_fn(|i| w[i] as f64);
                // (p - c)·(Z + f·W) = f·(X - c·W) for both axes, linear in (x, y).
                let row = |r: usize| [w[r * 4], w[r * 4 + 1], w[r * 4 + 3]];
                let (xr, yr, zr, wr) = (row(0), row(1), row(2), row(3));
                let eq = |p: f64, c: f64, ar: [f64; 3]| -> [f64; 3] { std::array::from_fn(|k| (p - c) * (zr[k] + f * wr[k]) - f * (ar[k] - c * wr[k])) };
                let e1 = eq(px, cx, xr);
                let e2 = eq(py, cy, yr);
                let det = e1[0] * e2[1] - e1[1] * e2[0];
                if det == 0.0 {
                    return None;
                }
                Some(((-e1[2] * e2[1] + e1[1] * e2[2]) / det, (-e1[0] * e2[2] + e1[2] * e2[0]) / det))
            }
        }
    }

    fn child_xform(&self, parent: &Xform, child: &Obj) -> Xform {
        let own = display_ref(child).map(|d| d.matrix).unwrap_or_default();
        match (three_d::matrix(child), parent) {
            (Some(m3), Xform::Flat(m)) => Xform::Deep(three_d::multiply(&three_d::from_affine(m), &m3)),
            (Some(m3), Xform::Deep(w)) => Xform::Deep(three_d::multiply(w, &m3)),
            (None, Xform::Deep(w)) => Xform::Deep(three_d::multiply(w, &three_d::from_affine(&own))),
            (None, Xform::Flat(m)) => Xform::Flat(own.then(m)),
        }
    }

    /// The world bounds test (`DO+0xc8..0xd4`, inclusive). For 3D content
    /// the cached bounds always pass (`FUN_14103bf20` forces ±5e6) and the
    /// point is tested against the local bounds after the inverse
    /// homography (`FUN_141039ee0`).
    fn in_bounds(&self, o: &Obj, x: &Xform) -> bool {
        let b = super::bounds::content(self.vm, o);
        match x {
            Xform::Deep(_) => self.local(x).is_some_and(|(lx, ly)| b[0] <= lx && lx <= b[2] && b[1] <= ly && ly <= b[3]),
            Xform::Flat(m) => {
                let corners = [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])].map(|(x, y)| m.apply(x, y));
                let (minx, maxx) = corners.iter().fold((f64::MAX, f64::MIN), |a, c| (a.0.min(c.0), a.1.max(c.0)));
                let (miny, maxy) = corners.iter().fold((f64::MAX, f64::MIN), |a, c| (a.0.min(c.1), a.1.max(c.1)));
                let (px, py) = self.point;
                minx <= px && px <= maxx && miny <= py && py <= maxy
            }
        }
    }

    /// Own geometry (`FUN_141025420` / bounds tests for other kinds).
    fn content(&self, o: &Obj, x: &Xform) -> bool {
        if !self.in_bounds(o, x) {
            return false;
        }
        let Some(d) = display_ref(o) else { return false };
        let (kind, character) = (d.kind, d.character);
        drop(d);
        match kind {
            DisplayKind::Shape => {
                let Some(c) = character else { return false };
                let file = player::state(self.vm).borrow().file(c.library);
                let Ok(Some(shape)) = file.shape(c.id as usize) else { return false };
                self.local(x).is_some_and(|(lx, ly)| shape_hit(&shape, lx, ly))
            }
            DisplayKind::Text => true,
            DisplayKind::Bitmap => true,
            _ => false,
        }
    }

    fn graphics_hit(&self, o: &Obj, x: &Xform) -> bool {
        let Some(g) = display_ref(o).and_then(|d| d.graphics.clone()) else { return false };
        self.in_bounds(o, x) && self.local(x).is_some_and(|(lx, ly)| super::graphics::contains(&g, lx, ly))
    }

    /// `FUN_141025940`: masks and hit areas (mode 1 ignores visibility).
    fn shape_tree(&self, o: &Obj, x: &Xform, mode: u8) -> bool {
        let Some(d) = display_ref(o) else { return false };
        if !d.visible && mode == 0 {
            return false;
        }
        drop(d);
        if self.content(o, x) || self.graphics_hit(o, x) {
            return true;
        }
        children(o).iter().any(|c| self.shape_tree(c, &self.child_xform(x, c), mode))
    }

    /// `FUN_141021290`.
    fn hit(&mut self, o: &Obj, x: Xform, cand: Option<Obj>, no_children: bool, parent_hit_area: bool) -> bool {
        let Some(d) = display_ref(o) else { return false };
        if d.hittest & 1 != 0 || !d.visible || self.script_masks.contains(o) {
            return false;
        }
        let interactive = matches!(d.kind, DisplayKind::Sprite | DisplayKind::MovieClip | DisplayKind::Text | DisplayKind::Button);
        let is_stage = self.stage.as_ref() == Some(o) || self.root.as_ref() == Some(o);
        let cand = if interactive && !no_children && d.mouse_enabled && !is_stage { Some(o.clone()) } else { cand };
        let (mouse_enabled, mouse_children, hit_area) = (d.mouse_enabled, d.mouse_children, d.hit_area.clone());
        let scroll = d.scroll_rect;
        let container = matches!(d.kind, DisplayKind::Sprite | DisplayKind::MovieClip | DisplayKind::Stage);
        drop(d);
        let mut x = x;
        if let Some(r) = scroll {
            let Some((lx, ly)) = self.local(&x) else { return false };
            if !(0.0 <= lx && lx <= r[2] && 0.0 <= ly && ly <= r[3]) {
                return false;
            }
            let shift = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: -r[0], ty: -r[1] };
            x = match x {
                Xform::Flat(m) => Xform::Flat(shift.then(&m)),
                Xform::Deep(w) => Xform::Deep(three_d::multiply(&w, &three_d::from_affine(&shift))),
            };
        }
        if container {
            let kids = children(o);
            let mut hit_any = false;
            let mut i = kids.len();
            while i > 0 {
                i -= 1;
                let child = &kids[i];
                let Some(cd) = display_ref(child) else { continue };
                if cd.clip_depth > 0 || !cd.visible {
                    continue;
                }
                let depth = cd.depth;
                let script_mask = cd.mask.clone();
                drop(cd);
                let saved = (self.over.clone(), self.inner.clone());
                let cx = self.child_xform(&x, child);
                let child_area = hit_area.is_some() && mouse_enabled;
                if !self.hit(child, cx, cand.clone(), no_children || !mouse_children, child_area) {
                    continue;
                }
                // Timeline masks covering the child.
                if let Some(depth) = depth {
                    let mask = kids.iter().enumerate().find(|(_, m)| {
                        display_ref(m).is_some_and(|md| md.clip_depth > 0 && md.depth.is_some_and(|d| d < depth) && md.clip_depth as i32 >= depth)
                    });
                    if let Some((mi, m)) = mask {
                        if !self.shape_tree(m, &self.child_xform(&x, m), 0) {
                            (self.over, self.inner) = saved;
                            i = mi;
                            continue;
                        }
                    }
                }
                if let Some(m) = script_mask {
                    let mx = Xform::Flat(super::display::concatenated(&m));
                    if !self.shape_tree(&m, &mx, 1) {
                        (self.over, self.inner) = saved;
                        continue;
                    }
                }
                if self.over != cand {
                    return true;
                }
                hit_any = true;
            }
            if hit_any {
                return true;
            }
        }
        if let Some(area) = &hit_area {
            if mouse_enabled && matches!(kind(o), Some(DisplayKind::Sprite | DisplayKind::MovieClip | DisplayKind::Text | DisplayKind::Button)) {
                let ax = Xform::Flat(super::display::concatenated(area));
                if self.shape_tree(area, &ax, 1) {
                    self.over = Some(o.clone());
                    self.inner = Some(o.clone());
                    return true;
                }
            }
        }
        if !parent_hit_area && hit_area.is_none() && cand.is_some() && self.content(o, &x) {
            self.over = cand.clone();
            self.inner = Some(o.clone());
            return true;
        }
        if cand.is_some() && (!container || mouse_enabled) && self.graphics_hit(o, &x) {
            self.over = cand;
            self.inner = Some(o.clone());
            return true;
        }
        false
    }
}

/// `FUN_141022460`: even-odd count over edges with exactly one empty side;
/// curves flattened (`FUN_1410203a0`, tolerance 1.0).
pub fn shape_hit(shape: &Shape, x: f64, y: f64) -> bool {
    let b = shape.bounds;
    if !(b[0] as f64 <= x && x <= b[2] as f64 && b[1] as f64 <= y && y <= b[3] as f64) {
        return false;
    }
    shape.layers.iter().any(|layer| {
        let mut inside = false;
        let mut pen = (0.0f64, 0.0f64);
        let mut cross = |a: (f64, f64), c: (f64, f64)| {
            if (a.1 > y) != (c.1 > y) {
                let t = (y - a.1) / (c.1 - a.1);
                if x < a.0 + t * (c.0 - a.0) {
                    inside = !inside;
                }
            }
        };
        for p in &layer.points {
            let end = (p.x as f64, p.y as f64);
            let edge = (p.fill0 == 0) != (p.fill1 == 0);
            match p.segment {
                Segment::MoveTo => {}
                Segment::LineTo => {
                    if edge {
                        cross(pen, end);
                    }
                }
                Segment::CurveTo => {
                    if edge {
                        let c = (p.cx as f64, p.cy as f64);
                        let chord = ((end.0 - pen.0).hypot(end.1 - pen.1) + (c.0 - pen.0).hypot(c.1 - pen.1) + (end.0 - c.0).hypot(end.1 - c.1)) * 0.5;
                        let steps = (chord.sqrt().ceil() as usize).clamp(1, 64);
                        let mut prev = pen;
                        for i in 1..=steps {
                            let t = i as f64 / steps as f64;
                            let u = 1.0 - t;
                            let q = (u * u * pen.0 + 2.0 * u * t * c.0 + t * t * end.0, u * u * pen.1 + 2.0 * u * t * c.1 + t * t * end.1);
                            cross(prev, q);
                            prev = q;
                        }
                    }
                }
            }
            pen = end;
        }
        inside
    })
}

/// `FUN_1410251c0`: the over object and innermost hit at the position.
fn pick(vm: &Vm) -> (Option<Obj>, Option<Obj>) {
    let shared = player::state(vm);
    let (stage, root, position, size, projection) = {
        let p = shared.borrow();
        (p.stage.clone(), p.root.clone(), p.mouse.position, p.stage_size, p.stage_projection)
    };
    let Some(stage) = stage else { return (None, None) };
    let mut script_masks = Vec::new();
    fn masks(o: &Obj, out: &mut Vec<Obj>) {
        for c in children(o) {
            if let Some(m) = display_ref(&c).and_then(|d| d.mask.clone()) {
                out.push(m);
            }
            masks(&c, out);
        }
    }
    masks(&stage, &mut script_masks);
    let mut picker = Picker {
        vm,
        point: position,
        projection: three_d::Projection::stage(size, projection.0, projection.1),
        stage: Some(stage.clone()),
        root: root.clone(),
        script_masks,
        over: None,
        inner: None,
    };
    picker.hit(&stage, Xform::Flat(Affine::default()), None, false, false);
    let over = picker.over.filter(|o| Some(o) != root.as_ref());
    (over, picker.inner)
}

fn event_target(vm: &Vm, o: &Option<Obj>) -> Value {
    o.clone().or_else(|| player::state(vm).borrow().stage.clone()).map_or(Value::Null, Value::Object)
}

/// `FUN_141086cc0`: a MouseEvent (cancelable 0) at the stage position.
fn dispatch(vm: &mut Vm, target: &Value, kind: &str, bubbles: bool, related: Option<Obj>, delta: i32) -> Result<()> {
    let (position, down, modifiers) = {
        let shared = player::state(vm);
        let p = shared.borrow();
        (p.mouse.position, p.mouse.button_down, p.modifiers)
    };
    let local = match target.as_object() {
        Some(o) if display_ref(o).is_some_and(|d| d.parent.is_some()) => super::display::concatenated(o).invert().apply(position.0, position.1),
        _ => position,
    };
    let related = related.map_or(Value::Null, Value::Object);
    let event = events::new_event(
        vm,
        "MouseEvent",
        &[
            kind.into(),
            bubbles.into(),
            false.into(),
            local.0.into(),
            local.1.into(),
            related,
            (modifiers.control() || modifiers.command()).into(),
            modifiers.alt().into(),
            modifiers.shift().into(),
            down.into(),
            (delta as f64).into(),
        ],
    )?;
    let result = events::dispatch(vm, target, event).map(|_| ());
    super::report(vm, result)
}

fn chain_of(o: &Option<Obj>, stage: &Option<Obj>) -> Vec<Obj> {
    let mut out = Vec::new();
    let mut cur = o.clone();
    while let Some(c) = cur {
        if Some(&c) == stage.as_ref() || out.len() == 100 {
            break;
        }
        cur = super::display::parent(&c);
        out.push(c);
    }
    out
}

/// `FUN_141026f40` / `FUN_141020dd0`: update the position, hit test, then
/// mouseMove (moves only), mouseOut, rollOut, rollOver, mouseOver.
pub fn moved(vm: &mut Vm, position: Option<(f64, f64)>) -> Result<()> {
    let shared = player::state(vm);
    if let Some(p) = position {
        shared.borrow_mut().mouse.position = p;
    }
    let (new, inner) = pick(vm);
    let (old, old_chain, stage) = {
        let mut p = shared.borrow_mut();
        let old = p.mouse.over.clone();
        p.mouse.over = new.clone();
        p.mouse.inner = inner;
        (old, p.mouse.chain.clone(), p.stage.clone())
    };
    if position.is_some() {
        let target = event_target(vm, &new);
        dispatch(vm, &target, "mouseMove", true, None, 0)?;
    }
    let enabled = |o: &Obj| display_ref(o).is_some_and(|d| d.mouse_enabled);
    let new_chain = chain_of(&new, &stage);
    let roll_over: Vec<Obj> = new_chain.iter().filter(|o| enabled(o) && !old_chain.contains(o)).cloned().collect();
    let old_live = old_chain.iter().find(|o| stage.as_ref().is_some_and(|s| super::display::root_of(o) == *s)).cloned();
    let mut roll_out = Vec::new();
    let mut cur = old_live;
    while let Some(c) = cur {
        if new_chain.contains(&c) || Some(&c) == stage.as_ref() {
            break;
        }
        if enabled(&c) {
            roll_out.push(c.clone());
        }
        cur = super::display::parent(&c);
    }
    shared.borrow_mut().mouse.chain = new_chain;
    if old.is_some() && old != new {
        let target = Value::Object(old.clone().unwrap());
        dispatch(vm, &target, "mouseOut", true, new.clone(), 0)?;
    }
    for o in roll_out.iter().rev() {
        dispatch(vm, &Value::Object(o.clone()), "rollOut", false, None, 0)?;
    }
    for o in &roll_over {
        dispatch(vm, &Value::Object(o.clone()), "rollOver", false, None, 0)?;
    }
    if new.is_some() && old != new {
        let target = Value::Object(new.clone().unwrap());
        dispatch(vm, &target, "mouseOver", true, old, 0)?;
    }
    Ok(())
}

/// `FUN_1410250d0`: mouseFocusChange (cancelable), focusOut, focusIn.
fn change_focus(vm: &mut Vm, new: Option<Obj>) -> Result<()> {
    let shared = player::state(vm);
    let old = shared.borrow().focus.clone();
    if old == new {
        return Ok(());
    }
    let target = event_target(vm, &old);
    let related = new.clone().map_or(Value::Null, Value::Object);
    let focus_event = |vm: &mut Vm, kind: &str, cancelable: bool| -> Result<Value> {
        let e = events::new_event(vm, "FocusEvent", &[kind.into(), true.into(), cancelable.into()])?;
        events::set_field(&e, "relatedObject", related.clone());
        Ok(e)
    };
    let change = focus_event(vm, "mouseFocusChange", true)?;
    let allowed = events::dispatch(vm, &target, change);
    let allowed = match allowed {
        Ok(a) => a,
        Err(e) => return super::report(vm, Err(e)),
    };
    if !allowed {
        return Ok(());
    }
    if let Some(o) = &old {
        let e = focus_event(vm, "focusOut", false)?;
        let r = events::dispatch(vm, &Value::Object(o.clone()), e).map(|_| ());
        super::report(vm, r)?;
    }
    shared.borrow_mut().focus = new.clone();
    if let Some(n) = &new {
        // relatedObject is the new focus itself (Iggy, 0x141025186).
        let e = focus_event(vm, "focusIn", false)?;
        let r = events::dispatch(vm, &Value::Object(n.clone()), e).map(|_| ());
        super::report(vm, r)?;
    }
    Ok(())
}

/// Mouse buttons (types 1..6, `FUN_141026b20`): 0 left, 1 right, 2 middle.
pub fn button(vm: &mut Vm, button: u8, down: bool, now: f64) -> Result<()> {
    let shared = player::state(vm);
    let over = shared.borrow().mouse.over.clone();
    let target = event_target(vm, &over);
    let target_obj = target.as_object().cloned();
    if button == 0 {
        shared.borrow_mut().mouse.button_down = down;
    }
    if button == 0 && down && shared.borrow().mouse.press == 0 {
        let is_stage = over.is_none();
        let focusable = is_stage
            || over.as_ref().is_some_and(|o| display_ref(o).is_some_and(|d| d.tab_enabled.unwrap_or(false) || d.kind == DisplayKind::Text));
        if focusable {
            change_focus(vm, if is_stage { None } else { over.clone() })?;
        }
        let mut p = shared.borrow_mut();
        p.mouse.press = 1;
        p.mouse.pressed = target_obj.clone();
    }
    let kind = match (button, down) {
        (0, true) => "mouseDown",
        (0, false) => "mouseUp",
        (1, true) => "rightMouseDown",
        (1, false) => "rightMouseUp",
        (_, true) => "middleMouseDown",
        (_, false) => "middleMouseUp",
    };
    dispatch(vm, &target, kind, true, None, 0)?;
    if down && button > 0 && shared.borrow().mouse.press == 0 {
        let mut p = shared.borrow_mut();
        p.mouse.press = button + 1;
        p.mouse.pressed = target_obj.clone();
    }
    if button == 0 {
        // The click / double-click state machine.
        let (state, same, within, press) = {
            let p = shared.borrow();
            (p.mouse.double_state, p.mouse.double_target == target_obj, now < p.mouse.double_time + DOUBLE_CLICK_SECONDS, p.mouse.press)
        };
        let mut click_check = !down;
        let set = |s: u8, reset_target: bool| {
            let mut p = shared.borrow_mut();
            p.mouse.double_state = s;
            if reset_target {
                p.mouse.double_target = target_obj.clone();
                p.mouse.double_time = now;
            }
        };
        match (state, down) {
            (0, true) if press == 1 => {
                set(1, true);
                return Ok(());
            }
            (1, false) => set(2, false),
            (2, true) => {
                if same {
                    set(3, false)
                } else {
                    set(1, true)
                }
                return Ok(());
            }
            (3, false) if same && within => {
                let pressed = shared.borrow().mouse.pressed.clone();
                if pressed.as_ref().is_some_and(|o| display_ref(o).is_some_and(|d| d.double_click)) {
                    shared.borrow_mut().mouse.press = 0;
                    dispatch(vm, &Value::Object(pressed.unwrap()), "doubleClick", true, None, 0)?;
                }
                set(0, false);
            }
            (3, false) => set(2, true),
            _ => {
                set(0, false);
                click_check = !down;
            }
        }
        if click_check {
            let (press, pressed) = {
                let p = shared.borrow();
                (p.mouse.press, p.mouse.pressed.clone())
            };
            if press == 1 && pressed == target_obj {
                dispatch(vm, &target, "click", true, None, 0)?;
            }
        }
    } else if !down {
        let (press, pressed) = {
            let p = shared.borrow();
            (p.mouse.press, p.mouse.pressed.clone())
        };
        if press == button + 1 && pressed == target_obj {
            dispatch(vm, &target, if button == 1 { "rightClick" } else { "middleClick" }, true, None, 0)?;
        }
    }
    if !down {
        shared.borrow_mut().mouse.press = 0;
    }
    Ok(())
}

/// `FUN_141027260`: mouseWheel on the over object, no hit test.
pub fn wheel(vm: &mut Vm, delta: i32) -> Result<()> {
    let over = player::state(vm).borrow().mouse.over.clone();
    let target = event_target(vm, &over);
    dispatch(vm, &target, "mouseWheel", true, None, delta)
}
