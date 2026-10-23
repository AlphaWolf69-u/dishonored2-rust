//! Point hit tests, `getObjectsUnderPoint` and dragging.
use super::avm2::builtins::{argument as arg, number_property};
use super::avm2::object::Obj;
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::display::{children, concatenated, display, display_ref, DisplayKind};
use super::player;
use crate::assets::iggy::shape::{Layer, Segment, Shape};

/// Quadratic curves are subdivided into this many segments for point tests.
const CURVE_STEPS: usize = 8;

/// Even-odd crossing test of one fill region: the edges that border fill
/// `f` on exactly one side (SWF edge model).
fn layer_contains(layer: &Layer, x: f64, y: f64) -> bool {
    let mut inside = vec![false; layer.fills.len() + 1];
    let mut pen = (0.0f64, 0.0f64);
    let cross = |a: (f64, f64), b: (f64, f64), f0: u16, f1: u16, inside: &mut Vec<bool>| {
        if f0 == f1 {
            return;
        }
        if (a.1 > y) != (b.1 > y) {
            let t = (y - a.1) / (b.1 - a.1);
            if x < a.0 + t * (b.0 - a.0) {
                for f in [f0, f1] {
                    if f != 0 && (f as usize) < inside.len() {
                        inside[f as usize] ^= true;
                    }
                }
            }
        }
    };
    for p in &layer.points {
        let end = (p.x as f64, p.y as f64);
        match p.segment {
            Segment::MoveTo => {}
            Segment::LineTo => cross(pen, end, p.fill0, p.fill1, &mut inside),
            Segment::CurveTo => {
                let c = (p.cx as f64, p.cy as f64);
                let mut prev = pen;
                for i in 1..=CURVE_STEPS {
                    let t = i as f64 / CURVE_STEPS as f64;
                    let u = 1.0 - t;
                    let q = (u * u * pen.0 + 2.0 * u * t * c.0 + t * t * end.0, u * u * pen.1 + 2.0 * u * t * c.1 + t * t * end.1);
                    cross(prev, q, p.fill0, p.fill1, &mut inside);
                    prev = q;
                }
            }
        }
        pen = end;
    }
    inside.iter().any(|&i| i)
}

pub fn shape_contains(shape: &Shape, x: f64, y: f64) -> bool {
    shape.layers.iter().any(|l| layer_contains(l, x, y))
}

/// Whether `o`'s own content (no children) covers local point (x, y).
fn content_hit(vm: &Vm, o: &Obj, x: f64, y: f64, shape_flag: bool) -> bool {
    let b = super::bounds::content(vm, o);
    if !(x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3]) {
        return false;
    }
    if !shape_flag {
        return true;
    }
    let (kind, character, graphics) = match display_ref(o) {
        Some(d) => (d.kind, d.character, d.graphics.clone()),
        None => return false,
    };
    if let Some(g) = graphics {
        if super::graphics::contains(&g, x, y) {
            return true;
        }
    }
    match (kind, character) {
        (DisplayKind::Shape, Some(c)) => {
            let file = player::state(vm).borrow().file(c.library);
            file.shape(c.id as usize).ok().flatten().is_some_and(|s| shape_contains(&s, x, y))
        }
        (DisplayKind::Shape, None) => false,
        _ => true,
    }
}

fn hit_tree(vm: &Vm, o: &Obj, sx: f64, sy: f64, shape_flag: bool) -> bool {
    if !display_ref(o).is_some_and(|d| d.visible) && shape_flag {
        return false;
    }
    let (x, y) = concatenated(o).invert().apply(sx, sy);
    if content_hit(vm, o, x, y, shape_flag) {
        return true;
    }
    children(o).iter().any(|c| hit_tree(vm, c, sx, sy, shape_flag))
}

/// `DisplayObject.hitTestPoint` with a stage-space point.
pub fn hit_test_point(vm: &Vm, o: &Obj, x: f64, y: f64, shape_flag: bool) -> bool {
    if !shape_flag {
        let stage = super::display::root_of(o);
        let b = super::bounds::bounds(vm, o, Some(&stage));
        return x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3];
    }
    hit_tree(vm, o, x, y, true)
}

/// Leaf display objects under a stage point, in render order, skipping
/// children whose hittest flags intersect `mask` (`FUN_14101ef60`).
pub fn objects_under_point_masked(vm: &Vm, container: &Obj, x: f64, y: f64, mask: u8) -> Vec<Obj> {
    let mut out = Vec::new();
    fn walk(vm: &Vm, o: &Obj, x: f64, y: f64, mask: u8, out: &mut Vec<Obj>) {
        for c in children(o) {
            if display_ref(&c).is_some_and(|d| d.hittest & mask != 0) {
                continue;
            }
            let (lx, ly) = concatenated(&c).invert().apply(x, y);
            if content_hit(vm, &c, lx, ly, true) {
                out.push(c.clone());
            }
            walk(vm, &c, x, y, mask, out);
        }
    }
    walk(vm, container, x, y, mask, &mut out);
    out
}

/// The topmost interactive object under a stage point (mouse targeting).
pub fn mouse_target(vm: &Vm, root: &Obj, x: f64, y: f64) -> Option<Obj> {
    fn walk(vm: &Vm, o: &Obj, x: f64, y: f64) -> Option<Obj> {
        let (visible, mouse_children) = display_ref(o).map(|d| (d.visible, d.mouse_children)).unwrap_or((false, false));
        if !visible {
            return None;
        }
        if mouse_children {
            for c in children(o).iter().rev() {
                if let Some(hit) = walk(vm, c, x, y) {
                    let enabled = display_ref(&hit).is_some_and(|d| d.mouse_enabled);
                    let interactive = display_ref(&hit).is_some_and(|d| d.kind != DisplayKind::Shape && d.kind != DisplayKind::Bitmap);
                    return Some(if enabled && interactive { hit } else { o.clone() });
                }
            }
        } else if children(o).iter().any(|c| hit_tree(vm, c, x, y, true)) {
            return Some(o.clone());
        }
        let (lx, ly) = concatenated(o).invert().apply(x, y);
        content_hit(vm, o, lx, ly, true).then(|| o.clone())
    }
    walk(vm, root, x, y)
}

pub fn start_drag(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let Some(o) = this.as_object().cloned() else { return Ok(Value::Undefined) };
    let lock_center = arg(a, 0).to_boolean();
    let bounds = match arg(a, 1) {
        Value::Object(_) => {
            let r = arg(a, 1);
            let x = number_property(vm, &r, "x")?;
            let y = number_property(vm, &r, "y")?;
            let w = number_property(vm, &r, "width")?;
            let h = number_property(vm, &r, "height")?;
            Some([x, y, x + w, y + h])
        }
        _ => None,
    };
    let shared = player::state(vm);
    let mut p = shared.borrow_mut();
    let (mx, my) = p.mouse.position;
    let parent = super::display::parent(&o);
    let (px, py) = parent.as_ref().map_or((mx, my), |pa| concatenated(pa).invert().apply(mx, my));
    let d = display_ref(&o).unwrap();
    let offset = if lock_center { (0.0, 0.0) } else { (d.matrix.tx - px, d.matrix.ty - py) };
    drop(d);
    p.drag = Some(super::player::Drag { object: o, offset, bounds });
    Ok(Value::Undefined)
}

pub fn stop_drag(vm: &mut Vm, _: &Value, _: &[Value]) -> Result<Value> {
    player::state(vm).borrow_mut().drag = None;
    Ok(Value::Undefined)
}

/// Move the dragged object to follow the pointer.
pub fn update_drag(vm: &Vm) {
    let shared = player::state(vm);
    let p = shared.borrow();
    let Some(drag) = &p.drag else { return };
    let (mx, my) = p.mouse.position;
    let parent = super::display::parent(&drag.object);
    let (px, py) = parent.as_ref().map_or((mx, my), |pa| concatenated(pa).invert().apply(mx, my));
    let (mut x, mut y) = (px + drag.offset.0, py + drag.offset.1);
    if let Some(b) = drag.bounds {
        x = x.clamp(b[0], b[2]);
        y = y.clamp(b[1], b[3]);
    }
    let object = drag.object.clone();
    drop(p);
    if let Some(mut d) = display(&object) {
        d.matrix.tx = x;
        d.matrix.ty = y;
    };
}
