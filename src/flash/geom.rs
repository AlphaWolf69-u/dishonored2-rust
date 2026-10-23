//! `flash.geom`: value classes whose behavior is specified by the Flash API.
use super::avm2::builtins::{argument as arg, define, native_class, number_property as num, set_number_property as set_num, Spec};
use super::avm2::object::{NativeClass, NativeFn};
use super::avm2::value::{self, Value};
use super::avm2::{Result, Vm};

fn n(vm: &mut Vm, a: &[Value], i: usize, default: f64) -> Result<f64> {
    match a.get(i) {
        Some(Value::Undefined) | None => Ok(default),
        Some(v) => vm.to_number(v),
    }
}

fn init_vars(vm: &mut Vm, this: &Value, a: &[Value], names: &[(&str, f64)]) -> Result<Value> {
    for (i, (name, default)) in names.iter().enumerate() {
        let v = n(vm, a, i, *default)?;
        set_num(vm, this, name, v)?;
    }
    Ok(Value::Undefined)
}

pub fn new_point(vm: &mut Vm, x: f64, y: f64) -> Result<Value> {
    let class = native_class(vm, "flash.geom", "Point");
    vm.construct_class(&class, &[x.into(), y.into()])
}

pub fn new_rectangle(vm: &mut Vm, x: f64, y: f64, w: f64, h: f64) -> Result<Value> {
    let class = native_class(vm, "flash.geom", "Rectangle");
    vm.construct_class(&class, &[x.into(), y.into(), w.into(), h.into()])
}

/// An affine matrix as Flash stores it: x' = a·x + c·y + tx, y' = b·x + d·y + ty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub tx: f64,
    pub ty: f64,
}

impl Default for Affine {
    fn default() -> Self {
        Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 }
    }
}

impl Affine {
    /// `self` applied after `inner` (Flash `inner.concat(self)`).
    pub fn then(&self, outer: &Affine) -> Affine {
        Affine {
            a: self.a * outer.a + self.b * outer.c,
            b: self.a * outer.b + self.b * outer.d,
            c: self.c * outer.a + self.d * outer.c,
            d: self.c * outer.b + self.d * outer.d,
            tx: self.tx * outer.a + self.ty * outer.c + outer.tx,
            ty: self.tx * outer.b + self.ty * outer.d + outer.ty,
        }
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.tx, self.b * x + self.d * y + self.ty)
    }

    pub fn invert(&self) -> Affine {
        let det = self.a * self.d - self.b * self.c;
        if det == 0.0 {
            // Flash leaves a non-invertible matrix as the identity-like
            // degenerate result with translation negated.
            return Affine { a: 0.0, b: 0.0, c: 0.0, d: 0.0, tx: -self.tx, ty: -self.ty };
        }
        let a = self.d / det;
        let b = -self.b / det;
        let c = -self.c / det;
        let d = self.a / det;
        Affine { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) }
    }
}

pub fn read_matrix(vm: &mut Vm, m: &Value) -> Result<Affine> {
    Ok(Affine {
        a: num(vm, m, "a")?,
        b: num(vm, m, "b")?,
        c: num(vm, m, "c")?,
        d: num(vm, m, "d")?,
        tx: num(vm, m, "tx")?,
        ty: num(vm, m, "ty")?,
    })
}

pub fn write_matrix(vm: &mut Vm, m: &Value, v: &Affine) -> Result<()> {
    set_num(vm, m, "a", v.a)?;
    set_num(vm, m, "b", v.b)?;
    set_num(vm, m, "c", v.c)?;
    set_num(vm, m, "d", v.d)?;
    set_num(vm, m, "tx", v.tx)?;
    set_num(vm, m, "ty", v.ty)
}

pub fn new_matrix(vm: &mut Vm, v: &Affine) -> Result<Value> {
    let class = native_class(vm, "flash.geom", "Matrix");
    vm.construct_class(&class, &[v.a.into(), v.b.into(), v.c.into(), v.d.into(), v.tx.into(), v.ty.into()])
}

/// Colour transform multipliers and offsets (r, g, b, a).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colour {
    pub mul: [f64; 4],
    pub add: [f64; 4],
}

impl Default for Colour {
    fn default() -> Self {
        Colour { mul: [1.0; 4], add: [0.0; 4] }
    }
}

impl Colour {
    /// `self` applied after `inner` (Flash `ColorTransform.concat`).
    pub fn then(&self, outer: &Colour) -> Colour {
        let mut r = Colour::default();
        for i in 0..4 {
            r.mul[i] = self.mul[i] * outer.mul[i];
            r.add[i] = self.add[i] * outer.mul[i] + outer.add[i];
        }
        r
    }
}

const COLOUR_FIELDS: [&str; 8] = [
    "redMultiplier",
    "greenMultiplier",
    "blueMultiplier",
    "alphaMultiplier",
    "redOffset",
    "greenOffset",
    "blueOffset",
    "alphaOffset",
];

pub fn read_colour(vm: &mut Vm, c: &Value) -> Result<Colour> {
    let mut r = Colour::default();
    for i in 0..4 {
        r.mul[i] = num(vm, c, COLOUR_FIELDS[i])?;
        r.add[i] = num(vm, c, COLOUR_FIELDS[i + 4])?;
    }
    Ok(r)
}

pub fn new_colour(vm: &mut Vm, c: &Colour) -> Result<Value> {
    let class = native_class(vm, "flash.geom", "ColorTransform");
    let args: Vec<Value> = c.mul.iter().chain(c.add.iter()).map(|v| Value::from(*v)).collect();
    vm.construct_class(&class, &args)
}

fn point_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("add", |vm, this, a| {
            let o = arg(a, 0);
            let (x, y) = (num(vm, this, "x")? + num(vm, &o, "x")?, num(vm, this, "y")? + num(vm, &o, "y")?);
            new_point(vm, x, y)
        }),
        ("subtract", |vm, this, a| {
            let o = arg(a, 0);
            let (x, y) = (num(vm, this, "x")? - num(vm, &o, "x")?, num(vm, this, "y")? - num(vm, &o, "y")?);
            new_point(vm, x, y)
        }),
        ("clone", |vm, this, _| {
            let (x, y) = (num(vm, this, "x")?, num(vm, this, "y")?);
            new_point(vm, x, y)
        }),
        ("equals", |vm, this, a| {
            let o = arg(a, 0);
            Ok((num(vm, this, "x")? == num(vm, &o, "x")? && num(vm, this, "y")? == num(vm, &o, "y")?).into())
        }),
        ("normalize", |vm, this, a| {
            let len = n(vm, a, 0, 0.0)?;
            let (x, y) = (num(vm, this, "x")?, num(vm, this, "y")?);
            let cur = (x * x + y * y).sqrt();
            if cur > 0.0 {
                set_num(vm, this, "x", x * len / cur)?;
                set_num(vm, this, "y", y * len / cur)?;
            }
            Ok(Value::Undefined)
        }),
        ("offset", |vm, this, a| {
            let (dx, dy) = (n(vm, a, 0, 0.0)?, n(vm, a, 1, 0.0)?);
            let (x, y) = (num(vm, this, "x")?, num(vm, this, "y")?);
            set_num(vm, this, "x", x + dx)?;
            set_num(vm, this, "y", y + dy)?;
            Ok(Value::Undefined)
        }),
        ("toString", |vm, this, _| {
            let (x, y) = (num(vm, this, "x")?, num(vm, this, "y")?);
            Ok(format!("(x={}, y={})", value::number_to_string(x), value::number_to_string(y)).into())
        }),
    ]
}

fn rect(vm: &mut Vm, v: &Value) -> Result<[f64; 4]> {
    Ok([num(vm, v, "x")?, num(vm, v, "y")?, num(vm, v, "width")?, num(vm, v, "height")?])
}

fn set_rect(vm: &mut Vm, v: &Value, r: [f64; 4]) -> Result<()> {
    set_num(vm, v, "x", r[0])?;
    set_num(vm, v, "y", r[1])?;
    set_num(vm, v, "width", r[2])?;
    set_num(vm, v, "height", r[3])
}

fn rectangle_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("clone", |vm, this, _| {
            let r = rect(vm, this)?;
            new_rectangle(vm, r[0], r[1], r[2], r[3])
        }),
        ("contains", |vm, this, a| {
            let r = rect(vm, this)?;
            let (x, y) = (n(vm, a, 0, 0.0)?, n(vm, a, 1, 0.0)?);
            Ok((x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]).into())
        }),
        ("containsPoint", |vm, this, a| {
            let r = rect(vm, this)?;
            let p = arg(a, 0);
            let (x, y) = (num(vm, &p, "x")?, num(vm, &p, "y")?);
            Ok((x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]).into())
        }),
        ("containsRect", |vm, this, a| {
            let r = rect(vm, this)?;
            let o = rect(vm, &arg(a, 0))?;
            Ok((o[0] >= r[0] && o[1] >= r[1] && o[0] + o[2] <= r[0] + r[2] && o[1] + o[3] <= r[1] + r[3]).into())
        }),
        ("equals", |vm, this, a| {
            let r = rect(vm, this)?;
            let o = rect(vm, &arg(a, 0))?;
            Ok((r == o).into())
        }),
        ("inflate", |vm, this, a| {
            let mut r = rect(vm, this)?;
            let (dx, dy) = (n(vm, a, 0, 0.0)?, n(vm, a, 1, 0.0)?);
            r = [r[0] - dx, r[1] - dy, r[2] + 2.0 * dx, r[3] + 2.0 * dy];
            set_rect(vm, this, r)?;
            Ok(Value::Undefined)
        }),
        ("intersection", |vm, this, a| {
            let r = rect(vm, this)?;
            let o = rect(vm, &arg(a, 0))?;
            let (x0, y0) = (r[0].max(o[0]), r[1].max(o[1]));
            let (x1, y1) = ((r[0] + r[2]).min(o[0] + o[2]), (r[1] + r[3]).min(o[1] + o[3]));
            if x1 <= x0 || y1 <= y0 {
                return new_rectangle(vm, 0.0, 0.0, 0.0, 0.0);
            }
            new_rectangle(vm, x0, y0, x1 - x0, y1 - y0)
        }),
        ("intersects", |vm, this, a| {
            let r = rect(vm, this)?;
            let o = rect(vm, &arg(a, 0))?;
            Ok(((r[0] + r[2]).min(o[0] + o[2]) > r[0].max(o[0]) && (r[1] + r[3]).min(o[1] + o[3]) > r[1].max(o[1])).into())
        }),
        ("isEmpty", |vm, this, _| {
            let r = rect(vm, this)?;
            Ok((r[2] <= 0.0 || r[3] <= 0.0).into())
        }),
        ("offset", |vm, this, a| {
            let r = rect(vm, this)?;
            let (dx, dy) = (n(vm, a, 0, 0.0)?, n(vm, a, 1, 0.0)?);
            set_rect(vm, this, [r[0] + dx, r[1] + dy, r[2], r[3]])?;
            Ok(Value::Undefined)
        }),
        ("setEmpty", |vm, this, _| {
            set_rect(vm, this, [0.0; 4])?;
            Ok(Value::Undefined)
        }),
        ("union", |vm, this, a| {
            let r = rect(vm, this)?;
            let o = rect(vm, &arg(a, 0))?;
            if o[2] <= 0.0 || o[3] <= 0.0 {
                return new_rectangle(vm, r[0], r[1], r[2], r[3]);
            }
            if r[2] <= 0.0 || r[3] <= 0.0 {
                return new_rectangle(vm, o[0], o[1], o[2], o[3]);
            }
            let (x0, y0) = (r[0].min(o[0]), r[1].min(o[1]));
            let (x1, y1) = ((r[0] + r[2]).max(o[0] + o[2]), (r[1] + r[3]).max(o[1] + o[3]));
            new_rectangle(vm, x0, y0, x1 - x0, y1 - y0)
        }),
        ("toString", |vm, this, _| {
            let r = rect(vm, this)?;
            Ok(format!(
                "(x={}, y={}, w={}, h={})",
                value::number_to_string(r[0]),
                value::number_to_string(r[1]),
                value::number_to_string(r[2]),
                value::number_to_string(r[3])
            )
            .into())
        }),
    ]
}

fn rectangle_accessors() -> (Vec<(&'static str, NativeFn)>, Vec<(&'static str, NativeFn)>) {
    let getters: Vec<(&'static str, NativeFn)> = vec![
        ("left", |vm, this, _| Ok(num(vm, this, "x")?.into())),
        ("top", |vm, this, _| Ok(num(vm, this, "y")?.into())),
        ("right", |vm, this, _| Ok((num(vm, this, "x")? + num(vm, this, "width")?).into())),
        ("bottom", |vm, this, _| Ok((num(vm, this, "y")? + num(vm, this, "height")?).into())),
        ("topLeft", |vm, this, _| {
            let r = rect(vm, this)?;
            new_point(vm, r[0], r[1])
        }),
        ("bottomRight", |vm, this, _| {
            let r = rect(vm, this)?;
            new_point(vm, r[0] + r[2], r[1] + r[3])
        }),
        ("size", |vm, this, _| {
            let r = rect(vm, this)?;
            new_point(vm, r[2], r[3])
        }),
    ];
    let setters: Vec<(&'static str, NativeFn)> = vec![
        // Moving an edge keeps the opposite edge fixed.
        ("left", |vm, this, a| {
            let r = rect(vm, this)?;
            let v = n(vm, a, 0, 0.0)?;
            set_rect(vm, this, [v, r[1], r[2] + r[0] - v, r[3]])?;
            Ok(Value::Undefined)
        }),
        ("top", |vm, this, a| {
            let r = rect(vm, this)?;
            let v = n(vm, a, 0, 0.0)?;
            set_rect(vm, this, [r[0], v, r[2], r[3] + r[1] - v])?;
            Ok(Value::Undefined)
        }),
        ("right", |vm, this, a| {
            let r = rect(vm, this)?;
            let v = n(vm, a, 0, 0.0)?;
            set_rect(vm, this, [r[0], r[1], v - r[0], r[3]])?;
            Ok(Value::Undefined)
        }),
        ("bottom", |vm, this, a| {
            let r = rect(vm, this)?;
            let v = n(vm, a, 0, 0.0)?;
            set_rect(vm, this, [r[0], r[1], r[2], v - r[1]])?;
            Ok(Value::Undefined)
        }),
    ];
    (getters, setters)
}

fn matrix_methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("clone", |vm, this, _| {
            let m = read_matrix(vm, this)?;
            new_matrix(vm, &m)
        }),
        ("concat", |vm, this, a| {
            let m = read_matrix(vm, this)?;
            let o = read_matrix(vm, &arg(a, 0))?;
            write_matrix(vm, this, &m.then(&o))?;
            Ok(Value::Undefined)
        }),
        ("identity", |vm, this, _| {
            write_matrix(vm, this, &Affine::default())?;
            Ok(Value::Undefined)
        }),
        ("invert", |vm, this, _| {
            let m = read_matrix(vm, this)?;
            write_matrix(vm, this, &m.invert())?;
            Ok(Value::Undefined)
        }),
        ("rotate", |vm, this, a| {
            let m = read_matrix(vm, this)?;
            let t = n(vm, a, 0, 0.0)?;
            let r = Affine { a: t.cos(), b: t.sin(), c: -t.sin(), d: t.cos(), tx: 0.0, ty: 0.0 };
            write_matrix(vm, this, &m.then(&r))?;
            Ok(Value::Undefined)
        }),
        ("scale", |vm, this, a| {
            let m = read_matrix(vm, this)?;
            let (sx, sy) = (n(vm, a, 0, 1.0)?, n(vm, a, 1, 1.0)?);
            let s = Affine { a: sx, b: 0.0, c: 0.0, d: sy, tx: 0.0, ty: 0.0 };
            write_matrix(vm, this, &m.then(&s))?;
            Ok(Value::Undefined)
        }),
        ("translate", |vm, this, a| {
            let mut m = read_matrix(vm, this)?;
            m.tx += n(vm, a, 0, 0.0)?;
            m.ty += n(vm, a, 1, 0.0)?;
            write_matrix(vm, this, &m)?;
            Ok(Value::Undefined)
        }),
        ("createBox", |vm, this, a| {
            let (sx, sy, r, tx, ty) = (n(vm, a, 0, 1.0)?, n(vm, a, 1, 1.0)?, n(vm, a, 2, 0.0)?, n(vm, a, 3, 0.0)?, n(vm, a, 4, 0.0)?);
            let m = Affine { a: r.cos() * sx, b: r.sin() * sy, c: -r.sin() * sx, d: r.cos() * sy, tx, ty };
            write_matrix(vm, this, &m)?;
            Ok(Value::Undefined)
        }),
        ("createGradientBox", |vm, this, a| {
            // Gradient space is the 1638.4-unit square mapped to the box.
            let (w, h, r, tx, ty) = (n(vm, a, 0, 0.0)?, n(vm, a, 1, 0.0)?, n(vm, a, 2, 0.0)?, n(vm, a, 3, 0.0)?, n(vm, a, 4, 0.0)?);
            let (sx, sy) = (w / 1638.4, h / 1638.4);
            let m = Affine { a: r.cos() * sx, b: r.sin() * sy, c: -r.sin() * sx, d: r.cos() * sy, tx: tx + w / 2.0, ty: ty + h / 2.0 };
            write_matrix(vm, this, &m)?;
            Ok(Value::Undefined)
        }),
        ("transformPoint", |vm, this, a| {
            let m = read_matrix(vm, this)?;
            let p = arg(a, 0);
            let (x, y) = m.apply(num(vm, &p, "x")?, num(vm, &p, "y")?);
            new_point(vm, x, y)
        }),
        ("deltaTransformPoint", |vm, this, a| {
            let mut m = read_matrix(vm, this)?;
            m.tx = 0.0;
            m.ty = 0.0;
            let p = arg(a, 0);
            let (x, y) = m.apply(num(vm, &p, "x")?, num(vm, &p, "y")?);
            new_point(vm, x, y)
        }),
        ("toString", |vm, this, _| {
            let m = read_matrix(vm, this)?;
            let s = value::number_to_string;
            Ok(format!("(a={}, b={}, c={}, d={}, tx={}, ty={})", s(m.a), s(m.b), s(m.c), s(m.d), s(m.tx), s(m.ty)).into())
        }),
    ]
}

pub fn install(vm: &mut Vm) {
    let object = vm.b().object.clone();
    let mut s = Spec::new("flash.geom", "Point", Some(object.clone()));
    s.vars = vec![("x", 0.into()), ("y", 0.into())];
    s.native = NativeClass { allocate: None, call: None, init: Some(|vm, this, a| init_vars(vm, this, a, &[("x", 0.0), ("y", 0.0)])) };
    s.methods = point_methods();
    s.getters = vec![("length", |vm, this, _| {
        let (x, y) = (num(vm, this, "x")?, num(vm, this, "y")?);
        Ok((x * x + y * y).sqrt().into())
    })];
    s.statics = vec![
        ("distance", |vm, _, a| {
            let (p, q) = (arg(a, 0), arg(a, 1));
            let (dx, dy) = (num(vm, &p, "x")? - num(vm, &q, "x")?, num(vm, &p, "y")? - num(vm, &q, "y")?);
            Ok((dx * dx + dy * dy).sqrt().into())
        }),
        ("interpolate", |vm, _, a| {
            let (p, q) = (arg(a, 0), arg(a, 1));
            let f = n(vm, a, 2, 0.0)?;
            let x = num(vm, &q, "x")? + f * (num(vm, &p, "x")? - num(vm, &q, "x")?);
            let y = num(vm, &q, "y")? + f * (num(vm, &p, "y")? - num(vm, &q, "y")?);
            new_point(vm, x, y)
        }),
        ("polar", |vm, _, a| {
            let (len, angle) = (n(vm, a, 0, 0.0)?, n(vm, a, 1, 0.0)?);
            new_point(vm, len * angle.cos(), len * angle.sin())
        }),
    ];
    define(vm, s);

    let mut s = Spec::new("flash.geom", "Rectangle", Some(object.clone()));
    s.vars = vec![("x", 0.into()), ("y", 0.into()), ("width", 0.into()), ("height", 0.into())];
    s.native = NativeClass {
        allocate: None,
        call: None,
        init: Some(|vm, this, a| init_vars(vm, this, a, &[("x", 0.0), ("y", 0.0), ("width", 0.0), ("height", 0.0)])),
    };
    s.methods = rectangle_methods();
    let (getters, setters) = rectangle_accessors();
    s.getters = getters;
    s.setters = setters;
    define(vm, s);

    let mut s = Spec::new("flash.geom", "Matrix", Some(object.clone()));
    s.vars = vec![("a", 1.into()), ("b", 0.into()), ("c", 0.into()), ("d", 1.into()), ("tx", 0.into()), ("ty", 0.into())];
    s.native = NativeClass {
        allocate: None,
        call: None,
        init: Some(|vm, this, a| init_vars(vm, this, a, &[("a", 1.0), ("b", 0.0), ("c", 0.0), ("d", 1.0), ("tx", 0.0), ("ty", 0.0)])),
    };
    s.methods = matrix_methods();
    define(vm, s);

    let mut s = Spec::new("flash.geom", "ColorTransform", Some(object.clone()));
    s.vars = COLOUR_FIELDS.iter().enumerate().map(|(i, f)| (*f, Value::from(if i < 4 { 1 } else { 0 }))).collect();
    s.native = NativeClass {
        allocate: None,
        call: None,
        init: Some(|vm, this, a| {
            let names: Vec<(&str, f64)> = COLOUR_FIELDS.iter().enumerate().map(|(i, f)| (*f, if i < 4 { 1.0 } else { 0.0 })).collect();
            init_vars(vm, this, a, &names)
        }),
    };
    s.getters = vec![("color", |vm, this, _| {
        let r = num(vm, this, "redOffset")? as u32 & 0xff;
        let g = num(vm, this, "greenOffset")? as u32 & 0xff;
        let b = num(vm, this, "blueOffset")? as u32 & 0xff;
        Ok(((r << 16) | (g << 8) | b).into())
    })];
    s.setters = vec![("color", |vm, this, a| {
        // Setting `color` zeroes the colour multipliers (alpha is kept).
        let c = vm.to_uint(&arg(a, 0))?;
        for (f, v) in [
            ("redMultiplier", 0.0),
            ("greenMultiplier", 0.0),
            ("blueMultiplier", 0.0),
            ("redOffset", ((c >> 16) & 0xff) as f64),
            ("greenOffset", ((c >> 8) & 0xff) as f64),
            ("blueOffset", (c & 0xff) as f64),
        ] {
            set_num(vm, this, f, v)?;
        }
        Ok(Value::Undefined)
    })];
    s.methods = vec![
        ("concat", |vm, this, a| {
            let c = read_colour(vm, this)?;
            let o = read_colour(vm, &arg(a, 0))?;
            let r = o.then(&c);
            for i in 0..4 {
                set_num(vm, this, COLOUR_FIELDS[i], r.mul[i])?;
                set_num(vm, this, COLOUR_FIELDS[i + 4], r.add[i])?;
            }
            Ok(Value::Undefined)
        }),
        ("toString", |vm, this, _| {
            let c = read_colour(vm, this)?;
            let s = value::number_to_string;
            Ok(format!(
                "(redMultiplier={}, greenMultiplier={}, blueMultiplier={}, alphaMultiplier={}, redOffset={}, greenOffset={}, blueOffset={}, alphaOffset={})",
                s(c.mul[0]), s(c.mul[1]), s(c.mul[2]), s(c.mul[3]), s(c.add[0]), s(c.add[1]), s(c.add[2]), s(c.add[3])
            )
            .into())
        }),
    ];
    define(vm, s);

    let mut s = Spec::new("flash.geom", "Vector3D", Some(object));
    s.vars = vec![("x", 0.into()), ("y", 0.into()), ("z", 0.into()), ("w", 0.into())];
    s.native = NativeClass {
        allocate: None,
        call: None,
        init: Some(|vm, this, a| init_vars(vm, this, a, &[("x", 0.0), ("y", 0.0), ("z", 0.0), ("w", 0.0)])),
    };
    define(vm, s);
}
