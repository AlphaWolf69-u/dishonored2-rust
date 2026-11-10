//! 14020a720: the strip built from a trail record each frame.
use super::{control_point, search, TrailRecord, EMPTY_BOUNDS};
use crate::{
    assets::fx::particle_trail::ParticleTrailAsset,
    float::{add, divide, multiply, subtract},
    gameplay::effects::{
        curve::{curve_key_times, EffectFloatCurve, EffectFloatCurvePair},
        vector_curve::{EffectVectorCurve, EffectVectorCurvePair},
    },
    gameplay::particles::parameters::VectorParameter,
};

/// Vertex format 4 (1402d0dc0 list +0x178), 40 bytes: POSITION f32x3,
/// TEXCOORD4 f16x4, NORMAL/TANGENT snorm8x4, COLOR unorm8x4, TEXCOORD5
/// f16x4. Trails never write TEXCOORD5; the original leaves whatever the
/// dynamic buffer held there, the rewrite writes zero.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct TrailVertex {
    pub position: [f32; 3],
    pub texcoord: [u16; 4],
    pub normal: u32,
    pub tangent: u32,
    pub colour: u32,
    pub texcoord5: [u16; 4],
}

/// 140210350's owner transform for stage-local trails: a row-major 3x4
/// whose fourth column is the owner position (140211a90).
pub type TrailTransform = [[f32; 4]; 3];

const UNIT: u32 = 0x3800_0100;

fn unit(seed: &mut u32) -> f32 {
    multiply(((crate::random::lcg_step(seed) >> 10) & 0x7fff) as f32, f32::from_bits(UNIT))
}

/// 14000ac60's truncating half conversion: underflow flushes to +0 and
/// overflow saturates at the largest finite half.
pub(crate) fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32 - 0x70;
    if exponent < 1 {
        0
    } else if exponent > 0x1e {
        sign | 0x7bff
    } else {
        sign | ((bits >> 13) & 0x3ff) as u16 | ((exponent as u16) << 10)
    }
}

/// 140212f90's signed byte: v*127 plus half away from zero, truncated.
fn snorm(value: f32) -> u32 {
    u32::from(add(multiply(value, 127.), 0.5_f32.copysign(value)) as i32 as u8)
}

/// 140164340.
fn unorm(colour: [f32; 4]) -> u32 {
    let byte = |v: f32| (multiply(v, 255.) as i32).clamp(0, 255) as u32;
    byte(colour[0]) | byte(colour[1]) << 8 | byte(colour[2]) << 16 | byte(colour[3]) << 24
}

/// Cursor search shared by 1402099b0/140209330: the cursor only moves
/// back, since strip samples go from older to newer segment ages. Returns
/// the interval and its normalized time, or the end key to return.
fn interval(times: &[f32], cursor: &mut usize, time: f32) -> Result<(usize, f32), usize> {
    let n = times.len();
    if n == 1 || !(times[0] < time) {
        return Err(0);
    }
    if !(time < times[n - 1]) {
        return Err(n - 1);
    }
    while *cursor > 0 && time < times[*cursor - 1] {
        *cursor -= 1;
    }
    let index = *cursor - 1;
    let [start, end] = curve_key_times(times, index as i32);
    let mut t = subtract(time, start);
    if f32::from_bits(0x3400_0000) < subtract(end, start) {
        t = divide(t, subtract(end, start));
    }
    Ok((index, t))
}

/// 1402099b0: a scalar curve pair along the strip; a minimum curve draws
/// one random mix.
fn strip_scalar(pair: &EffectFloatCurvePair, cursors: &mut [usize; 2], time: f32, seed: &mut u32) -> f32 {
    let sample = |curve: &EffectFloatCurve, cursor: &mut usize| {
        let n = curve.times.len();
        if n == 0 {
            return 0.;
        }
        let triples = curve.values.len() == n * 3;
        let at = |i: usize| if triples { i * 3 + 1 } else { i };
        match interval(&curve.times, cursor, time) {
            Err(key) => curve.values[at(key)],
            Ok((index, t)) => {
                let u = subtract(1., t);
                let a = multiply(multiply(t, 3.), multiply(u, u));
                let b = multiply(multiply(multiply(t, 3.), t), u);
                let c = multiply(multiply(t, t), t);
                let d = multiply(multiply(u, u), u);
                let v = &curve.values[at(index)..at(index) + 4];
                add(add(add(multiply(add(v[1], v[0]), a), multiply(v[0], d)), multiply(add(v[3], v[2]), b)), multiply(v[3], c))
            }
        }
    };
    let maximum = sample(&pair.maximum, &mut cursors[0]);
    if pair.minimum.times.is_empty() {
        return maximum;
    }
    let minimum = sample(&pair.minimum, &mut cursors[1]);
    add(multiply(unit(seed), subtract(maximum, minimum)), minimum)
}

/// 140209330: the vec3 counterpart; a minimum curve draws three mixes.
fn strip_vector(pair: &EffectVectorCurvePair<3>, cursors: &mut [usize; 2], time: f32, seed: &mut u32) -> [f32; 3] {
    let sample = |curve: &EffectVectorCurve<3>, cursor: &mut usize, minimum: bool| {
        let n = curve.times.len();
        if n == 0 {
            return [0.; 3];
        }
        let triples = curve.values.len() == n * 3;
        let at = |i: usize| if triples { i * 3 + 1 } else { i };
        match interval(&curve.times, cursor, time) {
            Err(key) => curve.values[at(key)],
            Ok((index, t)) => {
                let u = subtract(1., t);
                let a = multiply(multiply(multiply(t, 3.), u), u);
                let b = multiply(multiply(multiply(t, t), 3.), u);
                let c = multiply(multiply(t, t), t);
                let d = multiply(multiply(u, u), u);
                let p = &curve.values[at(index)..at(index) + 4];
                std::array::from_fn(|i| {
                    let (p0, p1, p2, p3) = (p[0][i], p[1][i], p[2][i], p[3][i]);
                    if minimum {
                        add(add(add(multiply(a, add(p1, p0)), multiply(d, p0)), multiply(b, add(p2, p3))), multiply(c, p3))
                    } else {
                        add(add(add(multiply(c, p3), multiply(b, add(p2, p3))), multiply(a, add(p0, p1))), multiply(p0, d))
                    }
                })
            }
        }
    };
    let maximum = sample(&pair.maximum, &mut cursors[0], false);
    if pair.minimum.times.is_empty() {
        return maximum;
    }
    let minimum = sample(&pair.minimum, &mut cursors[1], true);
    let mixes: [f32; 3] = std::array::from_fn(|_| ((crate::random::lcg_step(seed) >> 10) & 0x7fff) as f32);
    std::array::from_fn(|i| {
        add(minimum[i], multiply(multiply(subtract(maximum[i], minimum[i]), mixes[i]), f32::from_bits(UNIT)))
    })
}

/// 14020d440: the range part of a vec3 parameter only.
fn vector_range(parameter: &VectorParameter, seed: &mut u32) -> [f32; 3] {
    let [max, min] = parameter.range;
    if parameter.uniform {
        let f = unit(seed);
        return [
            add(min[0], multiply(f, subtract(max[0], min[0]))),
            add(min[1], multiply(subtract(max[1], min[1]), f)),
            add(min[2], multiply(f, subtract(max[2], min[2]))),
        ];
    }
    let n: [f32; 3] = std::array::from_fn(|_| ((crate::random::lcg_step(seed) >> 10) & 0x7fff) as f32);
    std::array::from_fn(|i| add(multiply(multiply(subtract(max[i], min[i]), n[i]), f32::from_bits(UNIT)), min[i]))
}

/// 140208ef0/140208eb0: [u0, v0, u1, v1, tangent basis x4, u step, v step]
/// plus the TEXCOORD4.zw halves written at mapping +0x28.
struct StripMapping {
    m: [f32; 10],
    extra: [u16; 2],
}
impl StripMapping {
    fn new(mapping: &crate::assets::fx::particle_trail::StripMapping, length: f32, segments: f32, flips: [bool; 2]) -> Self {
        let negative = |v: f32| f32::from_bits(v.to_bits() ^ 0x8000_0000);
        let mut scale = mapping.scale;
        if mapping.by_length {
            scale = multiply(scale, length);
        }
        let mut m = [0.; 10];
        if !mapping.along_v {
            m[4] = 1.;
            m[7] = 1.;
            if flips[0] {
                m[0] = scale;
                m[2] = scale;
                scale = negative(scale);
                m[4] = -1.;
                m[5] = -0.;
                m[6] = -0.;
                m[7] = -1.;
            }
            m[3] = 1.;
            if flips[1] {
                m[1] = 1.;
                m[3] = 0.;
            }
            m[8] = divide(scale, segments);
        } else {
            m[2] = 1.;
            m[5] = -1.;
            m[7] = 1.;
            if flips[0] {
                m[4] = -0.;
                m[5] = 1.;
                m[0] = 1.;
                m[2] = 0.;
                m[6] = -0.;
                m[7] = -1.;
            }
            if flips[1] {
                m[1] = scale;
                m[3] = scale;
                scale = negative(scale);
            }
            m[9] = divide(scale, segments);
        }
        Self { m, extra: [0; 2] }
    }
    fn advance(&mut self) {
        let m = &mut self.m;
        m[0] = add(m[0], m[8]);
        m[1] = add(m[9], m[1]);
        m[2] = add(m[2], m[8]);
        m[3] = add(m[9], m[3]);
    }
}

/// 14020f940: centre, unit tangent, side and normal, and |tangent . view|
/// for 14020f6f0's edge-on fade.
struct StripFrame {
    centre: [f32; 3],
    tangent: [f32; 3],
    side: [f32; 3],
    normal: [f32; 3],
    facing: f32,
}
impl StripFrame {
    fn new(centre: [f32; 3], tangent: [f32; 3], eye: [f32; 3]) -> Self {
        let unit = |v: [f32; 3], order: [usize; 3]| {
            let s = add(add(multiply(v[order[0]], v[order[0]]), multiply(v[order[1]], v[order[1]])), multiply(v[order[2]], v[order[2]]));
            let inverse = divide(1., s.max(f32::MIN_POSITIVE).sqrt());
            v.map(|c| multiply(c, inverse))
        };
        let t = unit(tangent, [0, 1, 2]);
        let v = unit(std::array::from_fn(|i| subtract(eye[i], centre[i])), [1, 0, 2]);
        let facing = add(add(multiply(t[1], v[1]), multiply(t[0], v[0])), multiply(t[2], v[2])).abs();
        let side = [
            subtract(multiply(t[2], v[1]), multiply(t[1], v[2])),
            subtract(multiply(t[0], v[2]), multiply(t[2], v[0])),
            subtract(multiply(t[1], v[0]), multiply(t[0], v[1])),
        ];
        let s = unit(side, [1, 0, 2]);
        let normal = [
            subtract(multiply(s[2], t[1]), multiply(s[1], t[2])),
            subtract(multiply(s[0], t[2]), multiply(s[2], t[0])),
            subtract(multiply(s[1], t[0]), multiply(s[0], t[1])),
        ];
        Self { centre, tangent: t, side: s, normal, facing }
    }
    fn fade(&self) -> f32 {
        let f = multiply(subtract(1., self.facing), 20.);
        if f <= 1. { f } else { 1. }
    }
}

/// 140212f90: (centre - width x side, centre + width x side).
fn vertex_pair(mapping: &StripMapping, f: &StripFrame, width: f32, colour: u32) -> [TrailVertex; 2] {
    let m = &mapping.m;
    let normal = snorm(f.normal[0]) | snorm(f.normal[1]) << 8 | snorm(f.normal[2]) << 16;
    let tx = add(add(multiply(m[5], f.side[0]), multiply(m[4], f.tangent[0])), multiply(m[6], f.normal[0]));
    let ty = add(add(multiply(m[4], f.tangent[1]), multiply(m[5], f.side[1])), multiply(m[6], f.normal[1]));
    let tz = add(add(multiply(m[4], f.tangent[2]), multiply(m[5], f.side[2])), multiply(m[6], f.normal[2]));
    let tangent = snorm(tx) | snorm(ty) << 8 | snorm(tz) << 16 | snorm(m[7]) << 24;
    let left = TrailVertex {
        position: std::array::from_fn(|i| subtract(f.centre[i], multiply(width, f.side[i]))),
        texcoord: [half(m[2]), half(m[3]), mapping.extra[0], mapping.extra[1]],
        normal,
        tangent,
        colour,
        texcoord5: [0; 4],
    };
    let right = TrailVertex {
        position: std::array::from_fn(|i| add(multiply(width, f.side[i]), f.centre[i])),
        texcoord: [half(m[0]), half(m[1]), mapping.extra[0], mapping.extra[1]],
        ..left
    };
    [left, right]
}

impl TrailRecord {
    /// 14020a720: append `segments + 1` vertex pairs, joined to an earlier
    /// strip by two degenerate vertices, and return the strip bounds (the
    /// record bounds grown by the widest width). The original grows the
    /// record's own bounds in place; they are rebuilt by every advance.
    pub fn build(
        &self,
        trail: &ParticleTrailAsset,
        segments: u32,
        eye: [f32; 3],
        tint: [f32; 4],
        transform: Option<&TrailTransform>,
        vertices: &mut Vec<TrailVertex>,
    ) -> Result<[[f32; 3]; 2], String> {
        if segments == 0 {
            return Ok(EMPTY_BOUNDS);
        }
        if trail.vector_oriented {
            return Err("vector-oriented particle trails are not built yet".into());
        }
        let mut seed = self.seed;
        let n = self.times.len();
        let mut time = subtract(self.time, self.life);
        if time <= self.times[0] {
            time = self.times[0];
        }
        let span = subtract(self.times[n - 1], time);
        let (life, segment) = (&trail.trail_life, &trail.segment_life);
        let colour = life.colour.sample(self.age, &mut seed)?;
        let range = vector_range(&segment.colour, &mut seed);
        let green = multiply(multiply(multiply(tint[1], self.colour[1]), colour[1]), range[1]);
        let blue = multiply(multiply(multiply(tint[2], self.colour[2]), colour[2]), range[2]);
        let alpha_mix = unit(&mut seed);
        let alpha_curve = life.alpha.curve.sample(self.age, unit(&mut seed))?;
        let segment_alpha_mix = unit(&mut seed);
        let segment_width_mix = unit(&mut seed);
        let width_mix = unit(&mut seed);
        let width_curve = life.width.curve.sample(self.age, unit(&mut seed))?;
        let lerp = |mix: f32, [max, min]: [f32; 2]| add(multiply(mix, subtract(max, min)), min);
        let width = multiply(
            multiply(multiply(lerp(segment_width_mix, segment.width.range), width_curve), lerp(width_mix, life.width.range)),
            self.width,
        );
        let mut widest = 0_f32;
        for curve in [&segment.width.curve.maximum, &segment.width.curve.minimum] {
            let triples = curve.values.len() == curve.times.len() * 3;
            for key in 0..curve.times.len() {
                let value = curve.values[if triples { key * 3 + 1 } else { key }].abs();
                if widest <= value {
                    widest = value;
                }
            }
        }
        let widest = multiply(widest, width);
        let mut bounds = self.bounds;
        for i in 0..3 {
            bounds[0][i] = subtract(bounds[0][i], widest);
            bounds[1][i] = add(widest, bounds[1][i]);
        }
        let mut mapping = StripMapping::new(&trail.mapping, self.length, segments as f32, self.flips);
        mapping.extra = [half(self.frame), half(unit(&mut seed))];
        let joint = (!vertices.is_empty()).then(|| {
            let previous = *vertices.last().unwrap();
            vertices.push(previous);
            vertices.push(previous);
            vertices.len() - 1
        });
        let (segment_alpha, trail_alpha) = (lerp(segment_alpha_mix, segment.alpha.range), lerp(alpha_mix, life.alpha.range));
        let red = multiply(multiply(multiply(tint[0], self.colour[0]), colour[0]), range[0]);
        let inverse_life = divide(1., self.life);
        let mut index = search(&self.times, time);
        let cursors = |pair_max: usize, pair_min: usize| [pair_max, pair_min];
        let mut colour_cursor = cursors(segment.colour.curve.maximum.times.len(), segment.colour.curve.minimum.times.len());
        let mut alpha_cursor = cursors(segment.alpha.curve.maximum.times.len(), segment.alpha.curve.minimum.times.len());
        let mut width_cursor = cursors(segment.width.curve.maximum.times.len(), segment.width.curve.minimum.times.len());
        for _ in 0..=segments {
            let age = multiply(subtract(self.time, time), inverse_life);
            let rgb = strip_vector(&segment.colour.curve, &mut colour_cursor, age, &mut seed);
            let alpha = strip_scalar(&segment.alpha.curve, &mut alpha_cursor, age, &mut seed);
            let packed = unorm([
                multiply(red, rgb[0]),
                multiply(rgb[1], green),
                multiply(rgb[2], blue),
                multiply(multiply(multiply(multiply(multiply(alpha, segment_alpha), alpha_curve), trail_alpha), tint[3]), self.colour[3]),
            ]);
            let segment_width = strip_scalar(&segment.width.curve, &mut width_cursor, age, &mut seed);
            while index < n && !(time <= self.times[index]) {
                index += 1;
            }
            let (mut centre, mut tangent) = self.centre_and_tangent(time, index as i64 - 1);
            if let Some(m) = transform {
                centre = std::array::from_fn(|r| {
                    add(add(add(m[r][3], multiply(m[r][1], centre[1])), multiply(m[r][2], centre[2])), multiply(m[r][0], centre[0]))
                });
                tangent = std::array::from_fn(|r| {
                    add(add(multiply(tangent[1], m[r][1]), multiply(tangent[0], m[r][0])), multiply(tangent[2], m[r][2]))
                });
            }
            let frame = StripFrame::new(centre, tangent, eye);
            let width = multiply(multiply(segment_width, width), frame.fade());
            vertices.extend(vertex_pair(&mapping, &frame, width, packed));
            mapping.advance();
            time = add(time, divide(span, segments as f32));
        }
        if let Some(joint) = joint {
            vertices[joint] = vertices[joint + 1];
        }
        Ok(bounds)
    }

    /// 140210240 -> 14020fb80 (centre) and 1401d1500 (tangent).
    fn centre_and_tangent(&self, time: f32, index: i64) -> ([f32; 3], [f32; 3]) {
        let [start, end] = curve_key_times(&self.times, index as i32);
        let interval = subtract(end, start);
        let mut t = subtract(time, start);
        if f32::from_bits(0x3400_0000) < interval {
            t = divide(t, interval);
        }
        let h = multiply(t, 0.5);
        let w0 = multiply(subtract(multiply(subtract(2., t), t), 1.), h);
        let w2 = multiply(add(multiply(subtract(4., multiply(t, 3.)), t), 1.), h);
        let w1 = add(multiply(multiply(subtract(multiply(t, 3.), 5.), t), h), 1.);
        let w3 = multiply(multiply(subtract(t, 1.), t), h);
        let p = [index - 1, index, index + 1, index + 2].map(|k| control_point(&self.positions, k));
        let centre = std::array::from_fn(|c| {
            add(add(add(multiply(p[1][c], w1), multiply(p[0][c], w0)), multiply(p[2][c], w2)), multiply(p[3][c], w3))
        });
        let d0 = subtract(multiply(subtract(2., multiply(t, 1.5)), t), 0.5);
        let d2 = add(multiply(subtract(4., multiply(t, 4.5)), t), 0.5);
        let d3 = subtract(multiply(multiply(t, 1.5), t), t);
        let d1 = multiply(subtract(multiply(t, 4.5), 5.), t);
        let scale = divide(1., add(f32::from_bits(0x3400_0000), interval));
        let tangent = std::array::from_fn(|c| {
            multiply(add(add(add(multiply(p[3][c], d3), multiply(p[2][c], d2)), multiply(p[1][c], d1)), multiply(p[0][c], d0)), scale)
        });
        (centre, tangent)
    }
}
