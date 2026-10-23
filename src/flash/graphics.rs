//! `flash.display.Graphics`: script drawing recorded as the same edge-path
//! layers file shapes use, so rendering and hit tests share one model.
use super::avm2::builtins::{argument as arg, define, Spec};
use super::avm2::object::{Kind, NativeClass, NativeFn, Obj};
use super::avm2::value::Value;
use super::avm2::{Result, Vm};
use super::Native;
use crate::assets::iggy::shape::{Fill, Layer, Line, PathPoint, Segment, Shape};

#[derive(Clone, Default)]
pub struct GraphicsState {
    pub layers: Vec<Layer>,
    fill: u16,
    line: u16,
    pen: (f32, f32),
    fill_start: Option<(f32, f32)>,
    /// Bitmap fills keep their BitmapData object.
    pub bitmaps: Vec<Value>,
    /// The library bitmap behind each script bitmap fill: (layer, fill
    /// number, character).
    pub bitmap_chars: Vec<(usize, u16, super::player::CharRef)>,
}

impl GraphicsState {
    fn layer(&mut self) -> &mut Layer {
        if self.layers.is_empty() {
            self.layers.push(Layer { bounds: [0.0; 4], points: Vec::new(), fills: Vec::new(), lines: Vec::new(), fill_bounds: Vec::new(), no_strokes: true });
        }
        self.layers.last_mut().unwrap()
    }

    fn point(&mut self, segment: Segment, x: f32, y: f32, c: (f32, f32)) {
        let (fill, line) = (self.fill, self.line);
        let fill = if self.fill_start.is_some() { fill } else { 0 };
        let layer = self.layer();
        if line != 0 {
            layer.no_strokes = false;
        }
        layer.points.push(PathPoint { x, y, cx: c.0, cy: c.1, segment, fill0: 0, fill1: fill, line });
        self.pen = (x, y);
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.point(Segment::MoveTo, x, y, (0.0, 0.0));
        if self.fill != 0 {
            self.fill_start = Some((x, y));
        }
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.point(Segment::LineTo, x, y, (0.0, 0.0));
    }

    /// Close the open fill back to its start (`endFill` / a new fill).
    fn close_fill(&mut self) {
        if let Some(start) = self.fill_start.take() {
            if self.pen != start {
                let line = self.line;
                self.line = 0;
                self.fill_start = Some(start);
                self.line_to(start.0, start.1);
                self.line = line;
                self.fill_start = None;
            }
        }
    }

    fn begin(&mut self, fill: Fill, bitmap: Option<Value>) {
        self.close_fill();
        let layer = self.layer();
        layer.fills.push(fill);
        layer.fill_bounds.push([0.0; 4]);
        self.fill = self.layer().fills.len() as u16;
        if let Some(b) = bitmap {
            let character = b.as_object().and_then(|o| o.host::<super::Native>()?.bitmap_data.as_ref()?.character);
            if let Some(c) = character {
                let at = (self.layers.len() - 1, self.fill, c);
                self.bitmap_chars.push(at);
            }
            self.bitmaps.push(b);
        }
        let pen = self.pen;
        self.fill_start = Some(pen);
    }

    pub fn shape(&self) -> Shape {
        Shape { bounds: self.bounds().map_or([0.0; 4], |b| b.map(|v| v as f32)), layers: self.layers.clone() }
    }

    fn bounds(&self) -> Option<[f64; 4]> {
        let mut b = super::bounds::EMPTY;
        for layer in &self.layers {
            for p in &layer.points {
                let half = if p.line != 0 { layer.lines.get(p.line as usize - 1).map_or(0.0, |l| l.width as f64 / 2.0) } else { 0.0 };
                let mut add = |x: f64, y: f64| {
                    b[0] = b[0].min(x - half);
                    b[1] = b[1].min(y - half);
                    b[2] = b[2].max(x + half);
                    b[3] = b[3].max(y + half);
                };
                add(p.x as f64, p.y as f64);
                if p.segment == Segment::CurveTo {
                    add(p.cx as f64, p.cy as f64);
                }
            }
        }
        (b[0] <= b[2]).then_some(b)
    }
}

pub fn state(o: &Obj) -> Option<std::cell::RefMut<'_, GraphicsState>> {
    std::cell::RefMut::filter_map(o.host_mut::<Native>()?, |n| n.graphics.as_mut()).ok()
}

pub fn bounds(g: &Obj) -> Option<[f64; 4]> {
    state(g)?.bounds()
}

pub fn contains(g: &Obj, x: f64, y: f64) -> bool {
    state(g).is_some_and(|s| super::hit::shape_contains(&s.shape(), x, y))
}

fn rgba(vm: &mut Vm, a: &[Value], colour: usize, alpha: usize) -> Result<[u8; 4]> {
    let c = vm.to_uint(&arg(a, colour))?;
    let alpha = match a.get(alpha) {
        Some(Value::Undefined) | None => 1.0,
        Some(v) => vm.to_number(v)?,
    };
    Ok([(c >> 16) as u8, (c >> 8) as u8, c as u8, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8])
}

fn with<R>(this: &Value, f: impl FnOnce(&mut GraphicsState) -> R) -> Option<R> {
    let o = this.as_object()?;
    let mut s = state(o)?;
    Some(f(&mut s))
}

fn n(vm: &mut Vm, a: &[Value], i: usize) -> Result<f32> {
    Ok(vm.to_number(&arg(a, i))? as f32)
}

fn methods() -> Vec<(&'static str, NativeFn)> {
    vec![
        ("clear", |_, this, _| {
            with(this, |s| *s = GraphicsState::default());
            Ok(Value::Undefined)
        }),
        ("beginFill", |vm, this, a| {
            let c = rgba(vm, a, 0, 1)?;
            with(this, |s| s.begin(Fill::Solid(c), None));
            Ok(Value::Undefined)
        }),
        ("beginBitmapFill", |vm, this, a| {
            let bitmap = arg(a, 0);
            let matrix = match arg(a, 1) {
                Value::Object(_) => {
                    let m = super::geom::read_matrix(vm, &arg(a, 1))?;
                    [m.a as f32, m.c as f32, m.b as f32, m.d as f32, m.tx as f32, m.ty as f32]
                }
                _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            };
            let repeat = a.get(2).is_none_or(|v| v.to_boolean());
            let smooth = arg(a, 3).to_boolean();
            with(this, |s| s.begin(Fill::Bitmap { matrix, bitmap: None, repeat, smooth }, Some(bitmap)));
            Ok(Value::Undefined)
        }),
        ("endFill", |_, this, _| {
            with(this, |s| {
                s.close_fill();
                s.fill = 0;
            });
            Ok(Value::Undefined)
        }),
        ("lineStyle", |vm, this, a| {
            if arg(a, 0).is_nullish() || vm.to_number(&arg(a, 0))?.is_nan() {
                with(this, |s| s.line = 0);
                return Ok(Value::Undefined);
            }
            let width = n(vm, a, 0)?;
            let c = rgba(vm, a, 1, 2)?;
            with(this, |s| {
                let layer = s.layer();
                layer.lines.push(Line { fill: Fill::Solid(c), width, multiplier: 1.0, scale_mode: 0 });
                s.line = s.layer().lines.len() as u16;
            });
            Ok(Value::Undefined)
        }),
        ("moveTo", |vm, this, a| {
            let (x, y) = (n(vm, a, 0)?, n(vm, a, 1)?);
            with(this, |s| s.move_to(x, y));
            Ok(Value::Undefined)
        }),
        ("lineTo", |vm, this, a| {
            let (x, y) = (n(vm, a, 0)?, n(vm, a, 1)?);
            with(this, |s| s.line_to(x, y));
            Ok(Value::Undefined)
        }),
        ("curveTo", |vm, this, a| {
            let (cx, cy, x, y) = (n(vm, a, 0)?, n(vm, a, 1)?, n(vm, a, 2)?, n(vm, a, 3)?);
            with(this, |s| s.point(Segment::CurveTo, x, y, (cx, cy)));
            Ok(Value::Undefined)
        }),
        ("drawRect", |vm, this, a| {
            let (x, y, w, h) = (n(vm, a, 0)?, n(vm, a, 1)?, n(vm, a, 2)?, n(vm, a, 3)?);
            with(this, |s| {
                s.move_to(x, y);
                s.line_to(x + w, y);
                s.line_to(x + w, y + h);
                s.line_to(x, y + h);
                s.line_to(x, y);
            });
            Ok(Value::Undefined)
        }),
        ("drawCircle", |vm, this, a| {
            let (x, y, r) = (n(vm, a, 0)?, n(vm, a, 1)?, n(vm, a, 2)?);
            with(this, |s| ellipse(s, x - r, y - r, 2.0 * r, 2.0 * r));
            Ok(Value::Undefined)
        }),
        ("drawEllipse", |vm, this, a| {
            let (x, y, w, h) = (n(vm, a, 0)?, n(vm, a, 1)?, n(vm, a, 2)?, n(vm, a, 3)?);
            with(this, |s| ellipse(s, x, y, w, h));
            Ok(Value::Undefined)
        }),
    ]
}

/// An ellipse as eight quadratic curves (the Flash Player construction).
fn ellipse(s: &mut GraphicsState, x: f32, y: f32, w: f32, h: f32) {
    let (rx, ry) = (w / 2.0, h / 2.0);
    let (cx, cy) = (x + rx, y + ry);
    let k = (std::f32::consts::PI / 8.0).tan();
    s.move_to(cx + rx, cy);
    for i in 1..=8 {
        let a = i as f32 * std::f32::consts::FRAC_PI_4;
        let m = a - std::f32::consts::FRAC_PI_8;
        let (ex, ey) = (cx + rx * a.cos(), cy + ry * a.sin());
        let r = (1.0 + k * k).sqrt();
        let (qx, qy) = (cx + rx * r * m.cos(), cy + ry * r * m.sin());
        s.point(Segment::CurveTo, ex, ey, (qx, qy));
    }
}

pub fn install(vm: &mut Vm) {
    let object = vm.b().object.clone();
    let mut s = Spec::new("flash.display", "Graphics", Some(object));
    s.native = NativeClass {
        allocate: Some(|_, _| Ok(Kind::Host(Box::new(Native { graphics: Some(GraphicsState::default()), ..Native::default() })))),
        call: None,
        init: None,
    };
    s.methods = methods();
    define(vm, s);
}
