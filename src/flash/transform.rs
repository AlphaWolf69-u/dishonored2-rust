//! `flash.geom.Transform`, `Matrix3D` and `PerspectiveProjection`.
use super::avm2::builtins::{argument as arg, define, native_class, number_property as num, set_number_property as set_num, Spec};
use super::avm2::object::{Kind, NativeClass, Obj, Vector};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::display::{concatenated, display, display_ref};
use super::geom::{self, Colour};
use super::Native;

/// Transform objects hold their display object.
fn target(vm: &mut Vm, this: &Value) -> Result<Obj> {
    let t = this.as_object().and_then(|o| o.host::<Native>().and_then(|n| n.target.clone()));
    t.ok_or_else(|| vm.type_error(1009, "Transform has no display object."))
}

/// `DisplayObject.transform`: a Transform bound to the object (its AS3
/// constructor throws TypeError, so natives allocate it directly).
pub fn new_transform(vm: &mut Vm, display_object: &Value) -> Result<Value> {
    let class = native_class(vm, "flash.geom", "Transform");
    let t = vm.allocate(&class)?;
    if let Some(mut n) = t.host_mut::<Native>() {
        n.target = display_object.as_object().cloned();
    }
    Ok(Value::Object(t))
}

/// `displayObject.transform = t` copies the transform's matrix and colour.
pub fn assign_transform(vm: &mut Vm, o: &Obj, t: &Value) -> Result<()> {
    let source = target(vm, t)?;
    let (m, c, m3) = {
        let d = display_ref(&source).unwrap();
        (d.matrix, d.colour, d.transform3d.clone())
    };
    let mut d = display(o).unwrap();
    d.set_matrix(m);
    d.colour = c;
    d.transform3d = m3;
    d.script_moved = true;
    Ok(())
}

fn number_vector(vm: &mut Vm, values: &[f64]) -> Result<Value> {
    let number = vm.b().number.clone();
    let class = vm.vector_class(Some(number))?;
    Ok(Value::Object(Obj::new(super::avm2::object::ObjectData {
        class: Some(class.clone()),
        traits: Some(class.instance.clone()),
        kind: Kind::Vector(Vector { element: class.applied.clone().flatten(), fixed: false, items: values.iter().map(|v| Value::from(*v)).collect() }),
        ..super::avm2::object::ObjectData::plain(Some(class.prototype()))
    })))
}

fn set_raw(_: &mut Vm, this: &Value, m: [f64; 16]) -> Result<()> {
    if let Some(mut n) = this.as_object().and_then(|o| o.host_mut::<Native>()) {
        n.matrix3d = Some(m);
    }
    Ok(())
}

const IDENTITY: [f64; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Column-major product `a * b` (applies `b` first).
fn multiply(a: &[f64; 16], b: &[f64; 16]) -> [f64; 16] {
    let mut r = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            r[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    r
}

fn rotation(degrees: f64, axis: [f64; 3]) -> [f64; 16] {
    let len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    let (x, y, z) = if len > 0.0 { (axis[0] / len, axis[1] / len, axis[2] / len) } else { (0.0, 0.0, 1.0) };
    let (s, c) = degrees.to_radians().sin_cos();
    let t = 1.0 - c;
    [
        t * x * x + c,
        t * x * y + s * z,
        t * x * z - s * y,
        0.0,
        t * x * y - s * z,
        t * y * y + c,
        t * y * z + s * x,
        0.0,
        t * x * z + s * y,
        t * y * z - s * x,
        t * z * z + c,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn vec3(vm: &mut Vm, v: &Value) -> Result<[f64; 3]> {
    Ok([num(vm, v, "x")?, num(vm, v, "y")?, num(vm, v, "z")?])
}

fn invert(m: &[f64; 16]) -> Option<[f64; 16]> {
    let mut inv = [0.0; 16];
    inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10];
    inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10];
    inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9];
    inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9];
    inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10];
    inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10];
    inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9];
    inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9];
    inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6];
    inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6];
    inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5];
    inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5];
    inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6];
    inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6];
    inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5];
    inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5];
    let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
    if det == 0.0 {
        return None;
    }
    Some(inv.map(|v| v / det))
}

/// The 4x4 matrix of a `Matrix3D` value.
pub fn matrix3d(_: &Vm, m: &Value) -> [f64; 16] {
    m.as_object().and_then(|o| o.host::<Native>().and_then(|n| n.matrix3d)).unwrap_or(IDENTITY)
}

fn with_raw(vm: &mut Vm, this: &Value, f: impl FnOnce(&mut Vm, [f64; 16]) -> Result<[f64; 16]>) -> Result<Value> {
    let m = matrix3d(vm, this);
    let r = f(vm, m)?;
    set_raw(vm, this, r)?;
    Ok(Value::Undefined)
}

pub fn install(vm: &mut Vm) {
    let object = vm.b().object.clone();

    let mut s = Spec::new("flash.geom", "Transform", Some(object.clone()));
    s.native = NativeClass {
        allocate: Some(|_, _| Ok(Kind::Host(Box::new(Native::default())))),
        call: None,
        init: None,
    };
    s.getters = vec![
        ("matrix", |vm, this, _| {
            let o = target(vm, this)?;
            if display_ref(&o).unwrap().transform3d.is_some() {
                return Ok(Value::Null);
            }
            let m = display_ref(&o).unwrap().matrix;
            geom::new_matrix(vm, &m)
        }),
        ("colorTransform", |vm, this, _| {
            let o = target(vm, this)?;
            let c = display_ref(&o).unwrap().colour;
            geom::new_colour(vm, &c)
        }),
        ("concatenatedMatrix", |vm, this, _| {
            let o = target(vm, this)?;
            let m = concatenated(&o);
            geom::new_matrix(vm, &m)
        }),
        ("concatenatedColorTransform", |vm, this, _| {
            let o = target(vm, this)?;
            let mut c = Colour::default();
            let mut cur = Some(o);
            while let Some(x) = cur {
                c = c.then(&display_ref(&x).unwrap().colour);
                cur = super::display::parent(&x);
            }
            geom::new_colour(vm, &c)
        }),
        ("matrix3D", |vm, this, _| {
            let o = target(vm, this)?;
            let v = display_ref(&o).unwrap().transform3d.clone();
            Ok(v.unwrap_or(Value::Null))
        }),
        // `0x14101ad80`: with the projection bit set (always on the root
        // timeline, `FUN_141029e30`), a new PerspectiveProjection linked to
        // the object; otherwise null.
        ("perspectiveProjection", |vm, this, _| {
            let o = target(vm, this)?;
            if object_projection(vm, &o).is_none() {
                return Ok(Value::Null);
            }
            let class = native_class(vm, "flash.geom", "PerspectiveProjection");
            let p = vm.construct_class(&class, &[])?;
            if let Some(mut n) = p.as_object().and_then(|p| p.host_mut::<Native>()) {
                n.target = Some(o);
            }
            Ok(p)
        }),
        ("pixelBounds", |vm, this, _| {
            let o = target(vm, this)?;
            let stage = super::display::root_of(&o);
            let b = super::bounds::bounds(vm, &o, Some(&stage));
            geom::new_rectangle(vm, b[0], b[1], b[2] - b[0], b[3] - b[1])
        }),
    ];
    s.setters = vec![
        ("matrix", |vm, this, a| {
            let o = target(vm, this)?;
            let m = geom::read_matrix(vm, &arg(a, 0))?;
            let mut d = display(&o).unwrap();
            // `0x14101b280` writes the 2D matrix only; a Matrix3D stays.
            d.set_matrix(m);
            d.script_moved = true;
            Ok(Value::Undefined)
        }),
        ("colorTransform", |vm, this, a| {
            let o = target(vm, this)?;
            let c = geom::read_colour(vm, &arg(a, 0))?;
            display(&o).unwrap().colour = c;
            Ok(Value::Undefined)
        }),
        // `0x14101b320`: null throws, a Matrix3D owned by another object
        // throws; the link copies only m3/m7 into tx/ty.
        ("matrix3D", |vm, this, a| {
            let o = target(vm, this)?;
            let v = arg(a, 0);
            let Some(m) = v.as_object().cloned() else { return Err(vm.iggy_error(0x78)) };
            let owner = m.host::<Native>().and_then(|n| n.target.clone());
            if owner.as_ref().is_some_and(|w| *w != o) {
                return Err(vm.iggy_error(0x72));
            }
            if let Some(mut n) = m.host_mut::<Native>() {
                n.target = Some(o.clone());
            }
            let raw = matrix3d(vm, &v);
            let mut d = display(&o).unwrap();
            d.transform3d = Some(v);
            d.matrix.tx = raw[12] as f32 as f64;
            d.matrix.ty = raw[13] as f32 as f64;
            d.script_moved = true;
            Ok(Value::Undefined)
        }),
        // `0x14101b480`: copies the projection's values (its own, or its
        // linked object's) and sets the bit; the object is not linked. A
        // value that is not a PerspectiveProjection throws (0x78); null
        // dereferences a null object in the game and is refused the same way.
        ("perspectiveProjection", |vm, this, a| {
            let o = target(vm, this)?;
            let v = arg(a, 0);
            let is_projection = v.as_object().is_some_and(|p| p.host::<Native>().is_some_and(|n| n.projection.is_some()));
            if !is_projection {
                return Err(vm.iggy_error(0x78));
            }
            let values = projection_values(vm, &v);
            set_object_projection(vm, &o, values);
            Ok(Value::Undefined)
        }),
    ];
    define(vm, s);

    let mut s = Spec::new("flash.geom", "Matrix3D", Some(object.clone()));
    s.native = NativeClass {
        allocate: Some(|_, _| Ok(Kind::Host(Box::new(Native::default())))),
        call: None,
        init: Some(|vm, this, a| {
            let mut m = IDENTITY;
            if let Some(Value::Object(v)) = a.first() {
                if let Kind::Vector(v) = &v.borrow().kind {
                    for (i, x) in v.items.iter().take(16).enumerate() {
                        m[i] = x.primitive_to_number();
                    }
                }
            }
            set_raw(vm, this, m)?;
            Ok(Value::Undefined)
        }),
    };
    s.getters = vec![
        ("rawData", |vm, this, _| {
            let m = matrix3d(vm, this);
            number_vector(vm, &m)
        }),
        ("position", |vm, this, _| {
            let m = matrix3d(vm, this);
            let class = native_class(vm, "flash.geom", "Vector3D");
            vm.construct_class(&class, &[m[12].into(), m[13].into(), m[14].into(), 0.into()])
        }),
    ];
    s.setters = vec![
        ("rawData", |vm, this, a| {
            let mut m = IDENTITY;
            if let Value::Object(v) = arg(a, 0) {
                if let Kind::Vector(v) = &v.borrow().kind {
                    for (i, x) in v.items.iter().take(16).enumerate() {
                        m[i] = x.primitive_to_number();
                    }
                }
            }
            set_raw(vm, this, m)?;
            Ok(Value::Undefined)
        }),
        ("position", |vm, this, a| {
            let p = vec3(vm, &arg(a, 0))?;
            with_raw(vm, this, |_, mut m| {
                m[12] = p[0];
                m[13] = p[1];
                m[14] = p[2];
                Ok(m)
            })
        }),
    ];
    s.methods = vec![
        ("identity", |vm, this, _| with_raw(vm, this, |_, _| Ok(IDENTITY))),
        ("clone", |vm, this, _| {
            let m = matrix3d(vm, this);
            let v = number_vector(vm, &m)?;
            let class = native_class(vm, "flash.geom", "Matrix3D");
            vm.construct_class(&class, &[v])
        }),
        ("append", |vm, this, a| {
            let lhs = matrix3d(vm, &arg(a, 0));
            with_raw(vm, this, |_, m| Ok(multiply(&lhs, &m)))
        }),
        ("prepend", |vm, this, a| {
            let rhs = matrix3d(vm, &arg(a, 0));
            with_raw(vm, this, |_, m| Ok(multiply(&m, &rhs)))
        }),
        ("appendTranslation", |vm, this, a| {
            let (x, y, z) = (vm.to_number(&arg(a, 0))?, vm.to_number(&arg(a, 1))?, vm.to_number(&arg(a, 2))?);
            with_raw(vm, this, |_, m| {
                let mut t = IDENTITY;
                t[12] = x;
                t[13] = y;
                t[14] = z;
                Ok(multiply(&t, &m))
            })
        }),
        ("prependTranslation", |vm, this, a| {
            let (x, y, z) = (vm.to_number(&arg(a, 0))?, vm.to_number(&arg(a, 1))?, vm.to_number(&arg(a, 2))?);
            with_raw(vm, this, |_, m| {
                let mut t = IDENTITY;
                t[12] = x;
                t[13] = y;
                t[14] = z;
                Ok(multiply(&m, &t))
            })
        }),
        ("appendScale", |vm, this, a| {
            let (x, y, z) = (vm.to_number(&arg(a, 0))?, vm.to_number(&arg(a, 1))?, vm.to_number(&arg(a, 2))?);
            with_raw(vm, this, |_, m| {
                let mut t = IDENTITY;
                t[0] = x;
                t[5] = y;
                t[10] = z;
                Ok(multiply(&t, &m))
            })
        }),
        ("appendRotation", |vm, this, a| {
            let degrees = vm.to_number(&arg(a, 0))?;
            let axis = vec3(vm, &arg(a, 1))?;
            with_raw(vm, this, |_, m| Ok(multiply(&rotation(degrees, axis), &m)))
        }),
        ("invert", |vm, this, _| {
            let m = matrix3d(vm, this);
            match invert(&m) {
                Some(i) => {
                    set_raw(vm, this, i)?;
                    Ok(true.into())
                }
                None => Ok(false.into()),
            }
        }),
    ];
    define(vm, s);

    // PerspectiveProjection (`0x141017e40` and the accessors after it): its
    // own values, or the linked object's read and written live.
    let mut s = Spec::new("flash.geom", "PerspectiveProjection", Some(object));
    s.native = NativeClass {
        // The constructor ignores its arguments: fov 55, centre (250, 250).
        allocate: Some(|_, _| Ok(Kind::Host(Box::new(Native { projection: Some([55.0, 250.0, 250.0]), ..Native::default() })))),
        call: None,
        init: None,
    };
    s.getters = vec![
        // `0x141017e80`: the raw f32.
        ("fieldOfView", |vm, this, _| Ok((projection_values(vm, this)[0] as f64).into())),
        // `0x141017ee0`: ((f32)W·0.5) / tan((f64)(fov·0.5)·3.141592/180).
        ("focalLength", |vm, this, _| {
            let fov = projection_values(vm, this)[0];
            let w = stage_width(vm) as f32;
            Ok(((w * 0.5) as f64 / ((fov * 0.5) as f64 * 3.141592 * (1.0 / 180.0)).tan()).into())
        }),
        // A Point copy of the centre.
        ("projectionCenter", |vm, this, _| {
            let [_, x, y] = projection_values(vm, this);
            geom::new_point(vm, x as f64, y as f64)
        }),
    ];
    s.setters = vec![
        // `0x141018020`: stored as f32, no range check.
        ("fieldOfView", |vm, this, a| {
            let v = vm.to_number(&arg(a, 0))? as f32;
            update_projection(vm, this, |p| p[0] = v);
            Ok(Value::Undefined)
        }),
        // `0x141018090`: fov = (f32)(atan(((f32)W·0.5) / v)·180·(1/π)·2).
        ("focalLength", |vm, this, a| {
            let v = vm.to_number(&arg(a, 0))?;
            let w = stage_width(vm) as f32;
            let fov = (((w * 0.5) as f64 / v).atan() * 180.0 * 0.3183099524062959 * 2.0) as f32;
            update_projection(vm, this, |p| p[0] = fov);
            Ok(Value::Undefined)
        }),
        ("projectionCenter", |vm, this, a| {
            let point = arg(a, 0);
            let (x, y) = (num(vm, &point, "x")? as f32, num(vm, &point, "y")? as f32);
            update_projection(vm, this, |p| {
                p[1] = x;
                p[2] = y;
            });
            Ok(Value::Undefined)
        }),
    ];
    define(vm, s);
    let _ = set_num;
}

/// `P+0x10`: the stage width as the integer the natives read.
fn stage_width(vm: &Vm) -> i32 {
    super::player::state(vm).borrow().stage_size.0 as i32
}

/// The root timeline, `P+0x2510` (the Stage, `P+0x26b0`, is its parent:
/// `FUN_141029f00`).
fn is_root(vm: &Vm, o: &Obj) -> bool {
    super::player::state(vm).borrow().root.as_ref() == Some(o)
}

/// A display object's projection (DO+0x188..0x190) when its bit is set. The
/// root timeline's is the projection the renderer uses (P+0x2698..0x26a0),
/// set at start-up with fov 55 and the centre at half the stage size
/// (`FUN_141029e30`).
fn object_projection(vm: &Vm, o: &Obj) -> Option<[f32; 3]> {
    if is_root(vm, o) {
        let state = super::player::state(vm);
        let p = state.borrow();
        let (w, h) = (p.stage_size.0 as i32, p.stage_size.1 as i32);
        let [x, y] = p.stage_projection.1.unwrap_or([w as f32 * 0.5, h as f32 * 0.5]);
        return Some([p.stage_projection.0, x, y]);
    }
    display_ref(o).and_then(|d| d.projection)
}

fn set_object_projection(vm: &Vm, o: &Obj, v: [f32; 3]) {
    if is_root(vm, o) {
        super::player::state(vm).borrow_mut().stage_projection = (v[0], Some([v[1], v[2]]));
    } else if let Some(mut d) = display(o) {
        d.projection = Some(v);
    }
}

/// A PerspectiveProjection's values: the linked object's, else its own.
fn projection_values(vm: &Vm, this: &Value) -> [f32; 3] {
    let Some(p) = this.as_object() else { return [55.0, 250.0, 250.0] };
    let (link, own) = match p.host::<Native>() {
        Some(n) => (n.target.clone(), n.projection),
        None => (None, None),
    };
    link.and_then(|o| object_projection(vm, &o)).or(own).unwrap_or([55.0, 250.0, 250.0])
}

fn update_projection(vm: &Vm, this: &Value, f: impl FnOnce(&mut [f32; 3])) {
    let Some(p) = this.as_object() else { return };
    let link = p.host::<Native>().and_then(|n| n.target.clone());
    let mut values = projection_values(vm, this);
    f(&mut values);
    match link {
        Some(o) => set_object_projection(vm, &o, values),
        None => {
            if let Some(mut n) = p.host_mut::<Native>() {
                n.projection = Some(values);
            }
        }
    }
}
