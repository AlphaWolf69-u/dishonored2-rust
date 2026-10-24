//! `flash.display` core: display objects, containers and the child list.
use super::avm2::builtins::{argument as arg, define, native_class, Spec};
use super::avm2::name::Name;
use super::avm2::object::{Class, Kind, NativeClass, NativeFn, Obj};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::events::{self, native};
use super::geom::{self, Affine, Colour};
use super::player::{self, CharRef};
use super::Native;
use std::cell::{Ref, RefMut};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    Shape,
    Sprite,
    MovieClip,
    Bitmap,
    Text,
    Button,
    Stage,
}

/// Timeline playback state of a sprite instance (instance +0x160..+0x17c).
#[derive(Clone)]
pub struct TimelineState {
    pub timeline: Rc<crate::assets::iggy::timeline::Timeline>,
    pub source: CharRef,
    /// 1-based current frame (instance +0x170 is 0-based).
    pub current: u32,
    pub playing: bool,
    /// `addFrameScript` functions by 0-based frame.
    pub scripts: Vec<(u32, Value)>,
    /// A frame script is pending for the current frame.
    pub script_pending: bool,
}

pub struct Display {
    pub kind: DisplayKind,
    pub name: Rc<str>,
    pub parent: Option<Obj>,
    pub children: Vec<Obj>,
    pub matrix: Affine,
    /// Cached decomposition used by the scaleX/scaleY/rotation accessors.
    pub scale: (f64, f64),
    pub rotation: f64,
    pub colour: Colour,
    pub visible: bool,
    /// Iggy blend code (1 normal).
    pub blend: u8,
    pub filters: Vec<Value>,
    /// The records `filters` was converted to (`FUN_141011300` copies each
    /// filter object's native record).
    pub filter_records: Vec<super::filter_passes::FilterRecord>,
    /// Filters placed by the timeline (until script assigns `filters`).
    pub timeline_filters: Vec<crate::assets::iggy::timeline::Filter>,
    pub mask: Option<Obj>,
    pub clip_depth: u16,
    /// Timeline depth; `None` for script-created children.
    pub depth: Option<i32>,
    /// Frame (0-based) at which the timeline created this instance (+0xc0).
    pub placed_frame: u32,
    pub character: Option<CharRef>,
    pub timeline: Option<TimelineState>,
    /// Script has written a transform: the timeline no longer moves it.
    pub script_moved: bool,
    pub ratio: f64,
    pub cache_as_bitmap: bool,
    pub scale9: Option<[f32; 4]>,
    pub scroll_rect: Option<[f64; 4]>,
    pub mouse_enabled: bool,
    pub mouse_children: bool,
    pub button_mode: bool,
    pub use_hand_cursor: bool,
    pub tab_enabled: Option<bool>,
    pub tab_index: i32,
    pub double_click: bool,
    /// `DO+0x78`.
    pub hit_area: Option<Obj>,
    pub graphics: Option<Obj>,
    pub transform3d: Option<Value>,
    /// `perspectiveProjection` fov and centre (DO+0x188..0x190), when set
    /// (DO+0xbc bit 3). The Stage's are the player's stage projection.
    pub projection: Option<[f32; 3]>,
    /// Constructed by the timeline but its class constructor has not run.
    pub constructed: bool,
    /// `IggyFunctions.setHittestProperties` flags (DO byte1 bits 4..6).
    pub hittest: u8,
    /// `IggyFunctions.setDepth` (DO +0x194 when 0xbb&0x80).
    pub custom_depth: Option<f32>,
    /// Cleared by `setObjectAntialiasingEnable(obj, false)` (0xbb&0x40).
    pub antialiasing: bool,
    /// A game texture bound by `IggyFunctions.setTextureForBitmap` (DO +0x148).
    pub game_texture: Option<String>,
}

impl Display {
    pub fn new(kind: DisplayKind) -> Self {
        Display {
            kind,
            name: "".into(),
            parent: None,
            children: Vec::new(),
            matrix: Affine::default(),
            scale: (1.0, 1.0),
            rotation: 0.0,
            colour: Colour::default(),
            visible: true,
            blend: 1,
            filters: Vec::new(),
            filter_records: Vec::new(),
            timeline_filters: Vec::new(),
            mask: None,
            clip_depth: 0,
            depth: None,
            placed_frame: 0,
            character: None,
            timeline: None,
            script_moved: false,
            ratio: 0.0,
            cache_as_bitmap: false,
            scale9: None,
            scroll_rect: None,
            mouse_enabled: true,
            mouse_children: true,
            button_mode: false,
            use_hand_cursor: true,
            tab_enabled: None,
            tab_index: -1,
            double_click: false,
            hit_area: None,
            graphics: None,
            transform3d: None,
            projection: None,
            constructed: true,
            hittest: 0,
            custom_depth: None,
            antialiasing: true,
            game_texture: None,
        }
    }

    pub fn set_matrix(&mut self, m: Affine) {
        self.matrix = m;
        let sx = (m.a * m.a + m.b * m.b).sqrt();
        let sy = (m.c * m.c + m.d * m.d).sqrt();
        let det = m.a * m.d - m.b * m.c;
        self.scale = (sx, if det < 0.0 { -sy } else { sy });
        self.rotation = m.b.atan2(m.a).to_degrees();
    }

    /// Rebuild the matrix from scale and rotation, keeping translation.
    pub fn rebuild_matrix(&mut self) {
        let r = self.rotation.to_radians();
        let (sx, sy) = self.scale;
        self.matrix = Affine { a: r.cos() * sx, b: r.sin() * sx, c: -r.sin() * sy, d: r.cos() * sy, tx: self.matrix.tx, ty: self.matrix.ty };
    }
}

pub fn display(o: &Obj) -> Option<RefMut<'_, Display>> {
    RefMut::filter_map(o.host_mut::<Native>()?, |n| n.display.as_mut()).ok()
}

pub fn display_ref(o: &Obj) -> Option<Ref<'_, Display>> {
    Ref::filter_map(o.host::<Native>()?, |n| n.display.as_ref()).ok()
}

fn this_obj(vm: &mut Vm, this: &Value) -> Result<Obj> {
    match this.as_object() {
        Some(o) if display_ref(o).is_some() => Ok(o.clone()),
        _ => Err(vm.type_error(1034, "receiver is not a DisplayObject.")),
    }
}

pub fn parent(o: &Obj) -> Option<Obj> {
    display_ref(o).and_then(|d| d.parent.clone())
}

pub fn children(o: &Obj) -> Vec<Obj> {
    display_ref(o).map(|d| d.children.clone()).unwrap_or_default()
}

/// The top of `o`'s parent chain.
pub fn root_of(o: &Obj) -> Obj {
    let mut cur = o.clone();
    while let Some(p) = parent(&cur) {
        cur = p;
    }
    cur
}

pub fn is_stage(o: &Obj) -> bool {
    display_ref(o).is_some_and(|d| d.kind == DisplayKind::Stage)
}

pub fn on_stage(o: &Obj) -> bool {
    is_stage(&root_of(o))
}

/// Matrix from `o`'s space to stage space.
pub fn concatenated(o: &Obj) -> Affine {
    let mut m = display_ref(o).map(|d| d.matrix).unwrap_or_default();
    let mut cur = parent(o);
    while let Some(p) = cur {
        if is_stage(&p) {
            break;
        }
        let pm = display_ref(&p).map(|d| d.matrix).unwrap_or_default();
        m = m.then(&pm);
        cur = parent(&p);
    }
    m
}

pub fn alloc(kind: DisplayKind) -> fn(&mut Vm, &Rc<Class>) -> Result<Kind> {
    fn make(vm: &mut Vm, class: &Rc<Class>, kind: DisplayKind) -> Result<Kind> {
        let mut d = Display::new(kind);
        if matches!(kind, DisplayKind::Sprite | DisplayKind::MovieClip | DisplayKind::Button) {
            // A class bound by SymbolClass instantiates its character
            // (`FUN_14102ad10` binding, `FUN_1410209c0` lookup).
            let qualified = class.name.qualified().replace("::", ".");
            let shared = player::state(vm);
            let p = shared.borrow();
            if let Some(c) = p.character_for_class(&qualified) {
                d.character = Some(c);
                if let Some(t) = p.timeline(c) {
                    d.scale9 = t.scale9;
                    d.timeline = Some(TimelineState { timeline: t, source: c, current: 0, playing: true, scripts: Vec::new(), script_pending: false });
                }
            }
        }
        Ok(Kind::Host(Box::new(Native { display: Some(d), ..Native::default() })))
    }
    match kind {
        DisplayKind::Shape => |vm, c| make(vm, c, DisplayKind::Shape),
        DisplayKind::Sprite => |vm, c| make(vm, c, DisplayKind::Sprite),
        DisplayKind::MovieClip => |vm, c| make(vm, c, DisplayKind::MovieClip),
        DisplayKind::Bitmap => |vm, c| make(vm, c, DisplayKind::Bitmap),
        DisplayKind::Text => |vm, c| make(vm, c, DisplayKind::Text),
        DisplayKind::Button => |vm, c| make(vm, c, DisplayKind::Button),
        DisplayKind::Stage => |vm, c| make(vm, c, DisplayKind::Stage),
    }
}

// ---- child list ----

fn dispatch_tree(vm: &mut Vm, o: &Obj, kind: &str) -> Result<()> {
    events::dispatch_simple(vm, &Value::Object(o.clone()), kind, false)?;
    for c in children(o) {
        dispatch_tree(vm, &c, kind)?;
    }
    Ok(())
}

/// Detach `child` from its parent with the Flash removal events.
pub fn detach(vm: &mut Vm, child: &Obj) -> Result<()> {
    let Some(p) = parent(child) else { return Ok(()) };
    events::dispatch_simple(vm, &Value::Object(child.clone()), "removed", true)?;
    if on_stage(child) {
        dispatch_tree(vm, child, "removedFromStage")?;
    }
    if let Some(mut d) = display(&p) {
        d.children.retain(|c| c != child);
    }
    if let Some(mut d) = display(child) {
        d.parent = None;
    }
    Ok(())
}

/// Insert `child` into `parent` at `index` with the Flash added events.
pub fn attach(vm: &mut Vm, parent_obj: &Obj, child: &Obj, index: usize) -> Result<()> {
    if parent(child).as_ref() == Some(parent_obj) {
        let mut d = display(parent_obj).unwrap();
        d.children.retain(|c| c != child);
        let i = index.min(d.children.len());
        d.children.insert(i, child.clone());
        return Ok(());
    }
    detach(vm, child)?;
    {
        let mut d = display(parent_obj).unwrap();
        let i = index.min(d.children.len());
        d.children.insert(i, child.clone());
    }
    display(child).unwrap().parent = Some(parent_obj.clone());
    events::dispatch_simple(vm, &Value::Object(child.clone()), "added", true)?;
    if on_stage(child) {
        dispatch_tree(vm, child, "addedToStage")?;
    }
    Ok(())
}

/// Link `child` into `parent` without events (timeline placement links
/// objects whose constructors are still pending, `FUN_1410221b0`).
pub fn link(parent_obj: &Obj, child: &Obj, index: usize) {
    if let Some(old) = parent(child) {
        if let Some(mut d) = display(&old) {
            d.children.retain(|c| c != child);
        }
    }
    {
        let mut d = display(parent_obj).unwrap();
        let i = index.min(d.children.len());
        d.children.insert(i, child.clone());
    }
    display(child).unwrap().parent = Some(parent_obj.clone());
}

/// `FUN_1410244c0`: `added`, then `addedToStage` when on the Stage.
pub fn dispatch_added(vm: &mut Vm, o: &Obj) -> Result<()> {
    events::dispatch_simple(vm, &Value::Object(o.clone()), "added", true)?;
    if on_stage(o) {
        events::dispatch_simple(vm, &Value::Object(o.clone()), "addedToStage", false)?;
    }
    Ok(())
}

/// Index for a timeline child at `depth`: before the first child whose
/// depth is greater (script children have no depth and stay on top).
pub fn depth_index(parent_obj: &Obj, depth: i32) -> usize {
    let kids = children(parent_obj);
    kids.iter().position(|c| display_ref(c).and_then(|d| d.depth).is_some_and(|d| d > depth)).unwrap_or_else(|| {
        // Above every timeline child but below script children placed later.
        kids.iter().rposition(|c| display_ref(c).and_then(|d| d.depth).is_some()).map_or(0, |i| i + 1)
    })
}

pub fn child_at_depth(parent_obj: &Obj, depth: i32) -> Option<Obj> {
    children(parent_obj).into_iter().find(|c| display_ref(c).and_then(|d| d.depth) == Some(depth))
}

fn child_arg(vm: &mut Vm, a: &[Value], i: usize) -> Result<Obj> {
    match arg(a, i) {
        Value::Object(o) if display_ref(&o).is_some() => Ok(o),
        _ => Err(vm.type_error(2007, "Parameter child must be non-null.")),
    }
}

fn index_arg(vm: &mut Vm, a: &[Value], i: usize, len: usize) -> Result<usize> {
    let n = vm.to_int(&arg(a, i))?;
    if n < 0 || n as usize > len {
        return Err(vm.range_error(2006, "The supplied index is out of bounds."));
    }
    Ok(n as usize)
}

fn container_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("addChild", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let c = child_arg(vm, a, 0)?;
            let len = children(&p).len();
            let len = if parent(&c).as_ref() == Some(&p) { len - 1 } else { len };
            attach(vm, &p, &c, len)?;
            Ok(Value::Object(c))
        }),
        ("addChildAt", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let c = child_arg(vm, a, 0)?;
            let len = children(&p).len();
            let i = index_arg(vm, a, 1, len)?;
            attach(vm, &p, &c, i)?;
            Ok(Value::Object(c))
        }),
        ("removeChild", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let c = child_arg(vm, a, 0)?;
            if parent(&c).as_ref() != Some(&p) {
                return Err(vm.argument_error(2025, "The supplied DisplayObject must be a child of the caller."));
            }
            detach(vm, &c)?;
            Ok(Value::Object(c))
        }),
        ("removeChildAt", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let kids = children(&p);
            let i = vm.to_int(&arg(a, 0))?;
            let Some(c) = (i >= 0).then(|| kids.get(i as usize).cloned()).flatten() else {
                return Err(vm.range_error(2006, "The supplied index is out of bounds."));
            };
            detach(vm, &c)?;
            Ok(Value::Object(c))
        }),
        ("getChildAt", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let i = vm.to_int(&arg(a, 0))?;
            match (i >= 0).then(|| children(&p).get(i as usize).cloned()).flatten() {
                Some(c) => Ok(Value::Object(c)),
                None => Err(vm.range_error(2006, "The supplied index is out of bounds.")),
            }
        }),
        ("getChildByName", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let name = vm.to_string(&arg(a, 0))?;
            Ok(children(&p).into_iter().find(|c| display_ref(c).is_some_and(|d| d.name == name)).map(Value::Object).unwrap_or(Value::Null))
        }),
        ("getChildIndex", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let c = child_arg(vm, a, 0)?;
            match children(&p).iter().position(|x| *x == c) {
                Some(i) => Ok((i as i32).into()),
                None => Err(vm.argument_error(2025, "The supplied DisplayObject must be a child of the caller.")),
            }
        }),
        ("setChildIndex", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let c = child_arg(vm, a, 0)?;
            let len = children(&p).len();
            let i = index_arg(vm, a, 1, len.saturating_sub(1))?;
            if parent(&c).as_ref() != Some(&p) {
                return Err(vm.argument_error(2025, "The supplied DisplayObject must be a child of the caller."));
            }
            attach(vm, &p, &c, i)?;
            Ok(Value::Undefined)
        }),
        ("swapChildren", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let (x, y) = (child_arg(vm, a, 0)?, child_arg(vm, a, 1)?);
            let mut d = display(&p).unwrap();
            let (Some(i), Some(j)) = (d.children.iter().position(|c| *c == x), d.children.iter().position(|c| *c == y)) else {
                drop(d);
                return Err(vm.argument_error(2025, "The supplied DisplayObject must be a child of the caller."));
            };
            d.children.swap(i, j);
            Ok(Value::Undefined)
        }),
        ("swapChildrenAt", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let len = children(&p).len();
            let i = index_arg(vm, a, 0, len.saturating_sub(1))?;
            let j = index_arg(vm, a, 1, len.saturating_sub(1))?;
            display(&p).unwrap().children.swap(i, j);
            Ok(Value::Undefined)
        }),
        ("contains", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let mut cur = arg(a, 0).as_object().cloned();
            while let Some(c) = cur {
                if c == p {
                    return Ok(true.into());
                }
                cur = parent(&c);
            }
            Ok(false.into())
        }),
        ("getObjectsUnderPoint", |vm, this, a| {
            let p = this_obj(vm, this)?;
            let pt = arg(a, 0);
            let x = super::avm2::builtins::number_property(vm, &pt, "x")?;
            let y = super::avm2::builtins::number_property(vm, &pt, "y")?;
            // Children flagged HITTEST_NO_GET_OBJECTS_UNDER_POINT (2) are skipped.
            let hits = super::hit::objects_under_point_masked(vm, &p, x, y, 2);
            Ok(Value::Object(vm.array(hits.into_iter().map(Value::Object).collect())))
        }),
    ]
}

// ---- transform accessors ----

fn set_moved(o: &Obj) -> RefMut<'_, Display> {
    let mut d = display(o).unwrap();
    d.script_moved = true;
    d
}

macro_rules! num_arg {
    ($vm:expr, $a:expr) => {
        $vm.to_number(&arg($a, 0))?
    };
}

fn display_getters() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("x", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().matrix.tx.into())),
        ("y", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().matrix.ty.into())),
        // 3D getters: 0 (scaleZ 1) for a 2D object (notes-3d.md section 1).
        ("z", |vm, this, _| Ok(super::three_d::matrix(&this_obj(vm, this)?).map_or(0.0, |m| m[11] as f64).into())),
        ("scaleX", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().scale.0.into())),
        ("scaleY", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().scale.1.into())),
        ("scaleZ", |vm, this, _| Ok(super::three_d::matrix(&this_obj(vm, this)?).map_or(1.0, |m| super::three_d::row_length(&m, 2) as f64).into())),
        ("rotation", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().rotation.into())),
        ("rotationX", |vm, this, _| Ok((super::three_d::angles(&this_obj(vm, this)?)[0] as f64).into())),
        ("rotationY", |vm, this, _| Ok((super::three_d::angles(&this_obj(vm, this)?)[1] as f64).into())),
        ("rotationZ", |vm, this, _| Ok((super::three_d::angles(&this_obj(vm, this)?)[2] as f64).into())),
        ("alpha", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().colour.mul[3].into())),
        ("visible", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().visible.into())),
        ("name", |vm, this, _| Ok(Value::String(display_ref(&this_obj(vm, this)?).unwrap().name.clone()))),
        ("parent", |vm, this, _| Ok(parent(&this_obj(vm, this)?).map(Value::Object).unwrap_or(Value::Null))),
        ("root", |vm, this, _| {
            // The movie root of an on-stage object; the topmost ancestor otherwise.
            let o = this_obj(vm, this)?;
            let r = root_of(&o);
            Ok(Value::Object(if is_stage(&r) { player::state(vm).borrow().root.clone().unwrap_or(r) } else { r }))
        }),
        ("stage", |vm, this, _| {
            let r = root_of(&this_obj(vm, this)?);
            Ok(if is_stage(&r) { Value::Object(r) } else { Value::Null })
        }),
        ("mask", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().mask.clone().map(Value::Object).unwrap_or(Value::Null))),
        ("filters", |vm, this, _| {
            let f = display_ref(&this_obj(vm, this)?).unwrap().filters.clone();
            Ok(Value::Object(vm.array(f)))
        }),
        ("blendMode", |vm, this, _| Ok(blend_name(display_ref(&this_obj(vm, this)?).unwrap().blend).into())),
        ("cacheAsBitmap", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().cache_as_bitmap.into())),
        ("width", |vm, this, _| {
            let o = this_obj(vm, this)?;
            let b = super::bounds::bounds(vm, &o, Some(&parent_space(&o)));
            Ok((b[2] - b[0]).into())
        }),
        ("height", |vm, this, _| {
            let o = this_obj(vm, this)?;
            let b = super::bounds::bounds(vm, &o, Some(&parent_space(&o)));
            Ok((b[3] - b[1]).into())
        }),
        ("mouseX", |vm, this, _| {
            let o = this_obj(vm, this)?;
            let (mx, my) = player::state(vm).borrow().mouse.position;
            Ok(concatenated(&o).invert().apply(mx, my).0.into())
        }),
        ("mouseY", |vm, this, _| {
            let o = this_obj(vm, this)?;
            let (mx, my) = player::state(vm).borrow().mouse.position;
            Ok(concatenated(&o).invert().apply(mx, my).1.into())
        }),
        ("transform", |vm, this, _| super::transform::new_transform(vm, this)),
        ("scale9Grid", |vm, this, _| {
            let s = display_ref(&this_obj(vm, this)?).unwrap().scale9;
            match s {
                Some(r) => geom::new_rectangle(vm, r[0] as f64, r[1] as f64, (r[2] - r[0]) as f64, (r[3] - r[1]) as f64),
                None => Ok(Value::Null),
            }
        }),
        ("scrollRect", |vm, this, _| {
            let s = display_ref(&this_obj(vm, this)?).unwrap().scroll_rect;
            match s {
                Some(r) => geom::new_rectangle(vm, r[0], r[1], r[2], r[3]),
                None => Ok(Value::Null),
            }
        }),
        ("loaderInfo", |_, _, _| Ok(Value::Null)),
    ]
}

/// Identity "space" for parent-relative bounds.
fn parent_space(o: &Obj) -> Obj {
    parent(o).unwrap_or_else(|| o.clone())
}

pub fn blend_name(code: u8) -> &'static str {
    match code {
        2 => "layer",
        3 => "multiply",
        4 => "screen",
        5 => "lighten",
        6 => "darken",
        7 => "add",
        8 => "subtract",
        9 => "difference",
        10 => "invert",
        11 => "alpha",
        12 => "erase",
        13 => "overlay",
        14 => "hardlight",
        _ => "normal",
    }
}

fn blend_code(name: &str) -> u8 {
    (1..=14).find(|&c| blend_name(c) == name).unwrap_or(1)
}

fn display_setters() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("x", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            if !v.is_nan() {
                set_moved(&o).matrix.tx = v;
                // Also m3 of an existing Matrix3D.
                if super::three_d::matrix_object(&o).is_some() {
                    super::three_d::update(vm, &o, |m, _| m[3] = v as f32)?;
                }
            }
            Ok(Value::Undefined)
        }),
        ("y", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            if !v.is_nan() {
                set_moved(&o).matrix.ty = v;
                // Also m7 of an existing Matrix3D.
                if super::three_d::matrix_object(&o).is_some() {
                    super::three_d::update(vm, &o, |m, _| m[7] = v as f32)?;
                }
            }
            Ok(Value::Undefined)
        }),
        // `0x1410122b0`: NaN becomes 0; m11 of the (created) Matrix3D.
        ("z", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            let v = if v.is_nan() { 0.0 } else { v as f32 };
            super::three_d::update(vm, &o, |m, _| m[11] = v)?;
            Ok(Value::Undefined)
        }),
        ("scaleX", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            {
                let mut d = set_moved(&o);
                d.scale.0 = v;
                d.rebuild_matrix();
            }
            // `FUN_14101f6b0`: row 0 of an existing Matrix3D to length v.
            if super::three_d::matrix_object(&o).is_some() {
                super::three_d::update(vm, &o, |m, _| super::three_d::set_row_length(m, 0, v as f32))?;
            }
            Ok(Value::Undefined)
        }),
        ("scaleY", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            {
                let mut d = set_moved(&o);
                d.scale.1 = v;
                d.rebuild_matrix();
            }
            // `FUN_14101f6b0`: row 1 of an existing Matrix3D to length v.
            if super::three_d::matrix_object(&o).is_some() {
                super::three_d::update(vm, &o, |m, _| super::three_d::set_row_length(m, 1, v as f32))?;
            }
            Ok(Value::Undefined)
        }),
        ("scaleZ", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a) as f32;
            super::three_d::update(vm, &o, |m, _| super::three_d::set_row_length(m, 2, v))?;
            Ok(Value::Undefined)
        }),
        ("rotation", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let mut v = num_arg!(vm, a) % 360.0;
            if v > 180.0 {
                v -= 360.0;
            } else if v < -180.0 {
                v += 360.0;
            }
            // `FUN_141011970`: on a 3D object this is the rotationZ setter.
            if super::three_d::matrix_object(&o).is_some() {
                super::three_d::set_rotation(vm, &o, 2, v as f32)?;
                return Ok(Value::Undefined);
            }
            let mut d = set_moved(&o);
            d.rotation = v;
            d.rebuild_matrix();
            Ok(Value::Undefined)
        }),
        // `FUN_14101f8e0` (axes 0/1/2) through thunks 0x141011b00/b10/b20.
        ("rotationZ", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a) as f32;
            super::three_d::set_rotation(vm, &o, 2, v)?;
            Ok(Value::Undefined)
        }),
        ("rotationX", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a) as f32;
            super::three_d::set_rotation(vm, &o, 0, v)?;
            Ok(Value::Undefined)
        }),
        ("rotationY", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a) as f32;
            super::three_d::set_rotation(vm, &o, 1, v)?;
            Ok(Value::Undefined)
        }),
        ("alpha", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            set_moved(&o).colour.mul[3] = v;
            Ok(Value::Undefined)
        }),
        ("visible", |vm, this, a| {
            let o = this_obj(vm, this)?;
            display(&o).unwrap().visible = arg(a, 0).to_boolean();
            Ok(Value::Undefined)
        }),
        ("name", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let n = vm.to_string(&arg(a, 0))?;
            display(&o).unwrap().name = n;
            Ok(Value::Undefined)
        }),
        ("mask", |vm, this, a| {
            let o = this_obj(vm, this)?;
            display(&o).unwrap().mask = arg(a, 0).as_object().cloned();
            Ok(Value::Undefined)
        }),
        ("filters", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let list = match arg(a, 0) {
                Value::Object(l) => match &l.borrow().kind {
                    Kind::Array(items) => items.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            // Filters are copied on assignment, as their records.
            let mut copies = Vec::with_capacity(list.len());
            let mut records = Vec::with_capacity(list.len());
            for f in list {
                records.push(filter_record(vm, &f)?);
                copies.push(vm.call_property(&f, &Name::public("clone"), &[]).unwrap_or(f));
            }
            let mut d = display(&o).unwrap();
            d.filters = copies;
            d.filter_records = records;
            Ok(Value::Undefined)
        }),
        ("blendMode", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let n = vm.to_string(&arg(a, 0))?;
            display(&o).unwrap().blend = blend_code(&n);
            Ok(Value::Undefined)
        }),
        ("cacheAsBitmap", |vm, this, a| {
            let o = this_obj(vm, this)?;
            display(&o).unwrap().cache_as_bitmap = arg(a, 0).to_boolean();
            Ok(Value::Undefined)
        }),
        ("width", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            set_extent(vm, &o, v, true);
            Ok(Value::Undefined)
        }),
        ("height", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = num_arg!(vm, a);
            set_extent(vm, &o, v, false);
            Ok(Value::Undefined)
        }),
        ("transform", |vm, this, a| {
            let o = this_obj(vm, this)?;
            super::transform::assign_transform(vm, &o, &arg(a, 0))?;
            Ok(Value::Undefined)
        }),
        ("scale9Grid", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let r = arg(a, 0);
            let grid = if r.is_nullish() {
                None
            } else {
                let x = super::avm2::builtins::number_property(vm, &r, "x")?;
                let y = super::avm2::builtins::number_property(vm, &r, "y")?;
                let w = super::avm2::builtins::number_property(vm, &r, "width")?;
                let h = super::avm2::builtins::number_property(vm, &r, "height")?;
                Some([x as f32, y as f32, (x + w) as f32, (y + h) as f32])
            };
            display(&o).unwrap().scale9 = grid;
            Ok(Value::Undefined)
        }),
        ("scrollRect", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let r = arg(a, 0);
            let rect = if r.is_nullish() {
                None
            } else {
                let mut out = [0.0; 4];
                for (i, n) in ["x", "y", "width", "height"].iter().enumerate() {
                    out[i] = super::avm2::builtins::number_property(vm, &r, n)?;
                }
                Some(out)
            };
            display(&o).unwrap().scroll_rect = rect;
            Ok(Value::Undefined)
        }),
    ]
}

/// The `width`/`height` setters scale to the requested parent-space extent.
fn set_extent(vm: &mut Vm, o: &Obj, value: f64, horizontal: bool) {
    let b = super::bounds::bounds(vm, o, None);
    let local = if horizontal { b[2] - b[0] } else { b[3] - b[1] };
    if local <= 0.0 || value.is_nan() {
        return;
    }
    let mut d = set_moved(o);
    if horizontal {
        d.scale.0 = value / local;
    } else {
        d.scale.1 = value / local;
    }
    d.rebuild_matrix();
}

fn display_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("localToGlobal", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let p = arg(a, 0);
            let x = super::avm2::builtins::number_property(vm, &p, "x")?;
            let y = super::avm2::builtins::number_property(vm, &p, "y")?;
            let (x, y) = concatenated(&o).apply(x, y);
            geom::new_point(vm, x, y)
        }),
        ("globalToLocal", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let p = arg(a, 0);
            let x = super::avm2::builtins::number_property(vm, &p, "x")?;
            let y = super::avm2::builtins::number_property(vm, &p, "y")?;
            let (x, y) = concatenated(&o).invert().apply(x, y);
            geom::new_point(vm, x, y)
        }),
        ("getBounds", bounds_method),
        ("getRect", bounds_method),
        ("hitTestPoint", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let x = vm.to_number(&arg(a, 0))?;
            let y = vm.to_number(&arg(a, 1))?;
            let shape = arg(a, 2).to_boolean();
            Ok(super::hit::hit_test_point(vm, &o, x, y, shape).into())
        }),
        ("hitTestObject", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let other = child_arg(vm, a, 0)?;
            let stage = root_of(&o);
            let x = super::bounds::bounds(vm, &o, Some(&stage));
            let y = super::bounds::bounds(vm, &other, Some(&stage));
            Ok((x[0] < y[2] && y[0] < x[2] && x[1] < y[3] && y[1] < x[3]).into())
        }),
    ]
}

fn bounds_method(vm: &mut Vm, this: &Value, a: &[Value]) -> Result<Value> {
    let o = this_obj(vm, this)?;
    let space = arg(a, 0).as_object().cloned();
    let b = super::bounds::bounds(vm, &o, space.as_ref());
    geom::new_rectangle(vm, b[0], b[1], b[2] - b[0], b[3] - b[1])
}

fn interactive_accessors() -> (Vec<(&'static str, NativeFn)>, Vec<(&'static str, NativeFn)>) {
    let getters: Vec<(&'static str, NativeFn)> = vec![
        ("mouseEnabled", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().mouse_enabled.into())),
        ("doubleClickEnabled", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().double_click.into())),
        ("tabEnabled", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().tab_enabled.unwrap_or(false).into())),
        ("tabIndex", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().tab_index.into())),
    ];
    let setters: Vec<(&'static str, NativeFn)> = vec![
        ("mouseEnabled", |vm, this, a| {
            display(&this_obj(vm, this)?).unwrap().mouse_enabled = arg(a, 0).to_boolean();
            Ok(Value::Undefined)
        }),
        ("doubleClickEnabled", |vm, this, a| {
            display(&this_obj(vm, this)?).unwrap().double_click = arg(a, 0).to_boolean();
            Ok(Value::Undefined)
        }),
        ("tabEnabled", |vm, this, a| {
            display(&this_obj(vm, this)?).unwrap().tab_enabled = Some(arg(a, 0).to_boolean());
            Ok(Value::Undefined)
        }),
        ("tabIndex", |vm, this, a| {
            let v = vm.to_int(&arg(a, 0))?;
            display(&this_obj(vm, this)?).unwrap().tab_index = v;
            Ok(Value::Undefined)
        }),
    ];
    (getters, setters)
}

fn container_accessors() -> (Vec<(&'static str, NativeFn)>, Vec<(&'static str, NativeFn)>) {
    let getters: Vec<(&'static str, NativeFn)> = vec![
        ("numChildren", |vm, this, _| Ok((children(&this_obj(vm, this)?).len() as u32).into())),
        ("mouseChildren", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().mouse_children.into())),
        ("tabChildren", |_, _, _| Ok(true.into())),
    ];
    let setters: Vec<(&'static str, NativeFn)> = vec![
        ("mouseChildren", |vm, this, a| {
            display(&this_obj(vm, this)?).unwrap().mouse_children = arg(a, 0).to_boolean();
            Ok(Value::Undefined)
        }),
        ("tabChildren", |_, _, _| Ok(Value::Undefined)),
    ];
    (getters, setters)
}

fn sprite_accessors() -> (Vec<(&'static str, NativeFn)>, Vec<(&'static str, NativeFn)>) {
    let getters: Vec<(&'static str, NativeFn)> = vec![
        ("graphics", graphics_getter),
        ("buttonMode", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().button_mode.into())),
        ("hitArea", |vm, this, _| Ok(display_ref(&this_obj(vm, this)?).unwrap().hit_area.clone().map_or(Value::Null, Value::Object))),
        ("dropTarget", |_, _, _| Ok(Value::Null)),
    ];
    let setters: Vec<(&'static str, NativeFn)> = vec![
        ("buttonMode", |vm, this, a| {
            display(&this_obj(vm, this)?).unwrap().button_mode = arg(a, 0).to_boolean();
            Ok(Value::Undefined)
        }),
        // DO+0x78: a Sprite or null.
        ("hitArea", |vm, this, a| {
            let o = this_obj(vm, this)?;
            let v = arg(a, 0).as_object().cloned();
            display(&o).unwrap().hit_area = v;
            Ok(Value::Undefined)
        }),
    ];
    (getters, setters)
}

/// `graphics` creates the object's Graphics on first use.
pub fn graphics_getter(vm: &mut Vm, this: &Value, _: &[Value]) -> Result<Value> {
    let o = this_obj(vm, this)?;
    if let Some(g) = display_ref(&o).unwrap().graphics.clone() {
        return Ok(Value::Object(g));
    }
    let class = native_class(vm, "flash.display", "Graphics");
    // Natives allocate without the (throwing) AS3 constructor.
    let g = vm.allocate(&class)?;
    display(&o).unwrap().graphics = Some(g.clone());
    Ok(Value::Object(g))
}

pub fn install(vm: &mut Vm) {
    let dispatcher = native_class(vm, "flash.events", "EventDispatcher");

    let mut s = Spec::new("flash.display", "DisplayObject", Some(dispatcher));
    s.native = NativeClass { allocate: Some(alloc(DisplayKind::Shape)), call: None, init: Some(|_, _, _| Ok(Value::Undefined)) };
    s.getters = display_getters();
    s.setters = display_setters();
    s.methods = display_methods();
    let display_object = define(vm, s);

    let mut s = Spec::new("flash.display", "InteractiveObject", Some(display_object.clone()));
    let (g, st) = interactive_accessors();
    s.getters = g;
    s.setters = st;
    let interactive = define(vm, s);

    let mut s = Spec::new("flash.display", "DisplayObjectContainer", Some(interactive.clone()));
    let (g, st) = container_accessors();
    s.getters = g;
    s.setters = st;
    s.methods = container_methods();
    let container = define(vm, s);

    let mut s = Spec::new("flash.display", "Sprite", Some(container.clone()));
    s.native = NativeClass { allocate: Some(alloc(DisplayKind::Sprite)), call: None, init: Some(super::timeline::construct_frame) };
    let (g, st) = sprite_accessors();
    s.getters = g;
    s.setters = st;
    s.methods = vec![
        ("startDrag", super::hit::start_drag),
        ("stopDrag", super::hit::stop_drag),
    ];
    let sprite = define(vm, s);

    super::timeline::install(vm, &sprite);

    let mut s = Spec::new("flash.display", "Shape", Some(display_object.clone()));
    s.native = NativeClass { allocate: Some(alloc(DisplayKind::Shape)), call: None, init: None };
    s.getters = vec![("graphics", graphics_getter)];
    define(vm, s);

    let mut s = Spec::new("flash.display", "SimpleButton", Some(interactive));
    s.native = NativeClass { allocate: Some(alloc(DisplayKind::Button)), call: None, init: None };
    s.getters = vec![
        ("upState", |_, _, _| Ok(Value::Null)),
        ("overState", |_, _, _| Ok(Value::Null)),
        ("downState", |_, _, _| Ok(Value::Null)),
        ("hitTestState", |_, _, _| Ok(Value::Null)),
        ("enabled", |_, _, _| Ok(true.into())),
    ];
    define(vm, s);

    let mut s = Spec::new("flash.display", "Stage", Some(container));
    s.native = NativeClass { allocate: Some(alloc(DisplayKind::Stage)), call: None, init: None };
    s.getters = vec![
        ("stageWidth", |vm, _, _| Ok(player::state(vm).borrow().stage_size.0.into())),
        ("stageHeight", |vm, _, _| Ok(player::state(vm).borrow().stage_size.1.into())),
        ("frameRate", |vm, _, _| Ok(player::state(vm).borrow().frame_rate.into())),
        ("focus", |vm, _, _| Ok(player::state(vm).borrow().focus.clone().map(Value::Object).unwrap_or(Value::Null))),
        ("quality", |_, _, _| Ok("HIGH".into())),
        ("scaleMode", |_, _, _| Ok("noScale".into())),
        ("align", |_, _, _| Ok("".into())),
        ("displayState", |_, _, _| Ok("normal".into())),
        ("stageFocusRect", |_, _, _| Ok(false.into())),
    ];
    s.setters = vec![
        ("focus", |vm, _, a| {
            player::state(vm).borrow_mut().focus = arg(a, 0).as_object().cloned();
            Ok(Value::Undefined)
        }),
        ("quality", |_, _, _| Ok(Value::Undefined)),
        ("scaleMode", |_, _, _| Ok(Value::Undefined)),
        ("align", |_, _, _| Ok(Value::Undefined)),
        ("stageFocusRect", |_, _, _| Ok(Value::Undefined)),
        ("frameRate", |_, _, _| Ok(Value::Undefined)),
    ];
    s.methods = vec![("invalidate", |_, _, _| Ok(Value::Undefined))];
    define(vm, s);

    let _ = native;
}

/// Indented display-tree listing for diagnostics.
pub fn debug_tree(o: &Obj, depth: usize, out: &mut String) {
    use std::fmt::Write;
    if let Some(d) = display_ref(o) {
        let class = o.class().map_or("?".to_string(), |c| c.name.qualified());
        let _ = writeln!(
            out,
            "{}{} name={:?} depth={:?} char={:?} kind={:?} frame={} visible={} alpha={} clip={} mask={}",
            "  ".repeat(depth),
            class,
            d.name,
            d.depth,
            d.character.map(|c| (c.library, c.id)),
            d.kind,
            d.timeline.as_ref().map_or(0, |t| t.current),
            d.visible,
            d.colour.mul[3],
            d.clip_depth,
            d.mask.is_some()
        );
    }
    for c in children(o) {
        debug_tree(&c, depth + 1, out);
    }
}

/// A filter object's record: the `flash.filters` class it is, with the
/// fields its native setters store (blur floats, the quality byte of
/// `FUN_1410127a0`, the matrix of `FUN_14100d950`).
fn filter_record(vm: &mut Vm, f: &Value) -> Result<super::filter_passes::FilterRecord> {
    use super::filter_passes::{colour_matrix, FilterRecord};
    let mut class = f.as_object().and_then(|o| o.class());
    while let Some(c) = class.clone() {
        if c.name.ns.uri() == "flash.filters" {
            break;
        }
        class = c.super_class.clone();
    }
    let name = class.map(|c| c.name.name.to_string()).unwrap_or_default();
    let num = |vm: &mut Vm, field: &str| -> Result<f64> {
        let v = vm.get_property(f, &Name::public(field))?;
        vm.to_number(&v)
    };
    Ok(match name.as_str() {
        "BlurFilter" => {
            let quality = vm.get_property(f, &Name::public("quality"))?;
            FilterRecord::Blur { blur: [num(vm, "blurX")? as f32, num(vm, "blurY")? as f32], passes: vm.to_int(&quality)? as u8 }
        }
        "DropShadowFilter" => {
            let colour = super::avm2::value::to_uint32(num(vm, "color")?);
            // `FUN_141012600`: below 0 is 0, above 1 is 255, else a × 255.5.
            let alpha = num(vm, "alpha")? as f32;
            let alpha = if alpha < 0.0 { 0 } else if alpha > 1.0 { 255 } else { (alpha * 255.5) as i32 as u8 };
            let flag = |vm: &mut Vm, field: &str| -> Result<bool> { Ok(vm.get_property(f, &Name::public(field))?.to_boolean()) };
            let quality = vm.get_property(f, &Name::public("quality"))?;
            FilterRecord::DropShadow {
                colour: [(colour >> 16) as u8, (colour >> 8) as u8, colour as u8, alpha],
                blur: [num(vm, "blurX")? as f32, num(vm, "blurY")? as f32],
                // `FUN_141012690`: degrees × π × (1/180), in single precision.
                angle: num(vm, "angle")? as f32 * std::f32::consts::PI * (1.0f32 / 180.0),
                distance: num(vm, "distance")? as f32,
                strength: num(vm, "strength")? as f32,
                passes: vm.to_int(&quality)? as u8,
                inner: flag(vm, "inner")?,
                knockout: flag(vm, "knockout")?,
                hide_object: flag(vm, "hideObject")?,
            }
        }
        "ColorMatrixFilter" => {
            let matrix = vm.get_property(f, &Name::public("matrix"))?;
            let items = match &matrix {
                Value::Object(o) => match &o.borrow().kind {
                    Kind::Array(items) => items.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            // Numbers convert to f32, booleans to 0/1, anything else to 0.
            let values: Vec<f32> = items
                .iter()
                .take(20)
                .map(|v| match v {
                    Value::Number(n) => *n as f32,
                    Value::Bool(b) => *b as i32 as f32,
                    _ => 0.0,
                })
                .collect();
            FilterRecord::ColourMatrix(colour_matrix(&values))
        }
        _ => FilterRecord::Unsupported,
    })
}
