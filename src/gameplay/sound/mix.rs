//! Wwise version-113 property curves and the authored output-bus hierarchy.
use crate::assets::wwise::{BankMix, MediaProperties, NodeProperties, RtpcCurve};
use std::collections::{HashMap, HashSet};

/// The logarithm approximation in conversion-table scaling (1410cb000).
fn ln(value: f32) -> f32 {
    let bits = value.to_bits();
    let mantissa = f32::from_bits((bits & 0x7fffff) + 0x3f800000);
    let t = (mantissa - 1.0) / (mantissa + 1.0);
    (t * t * 0.33333334 + 1.0) * t * 2.0 + (((bits >> 23) & 0xff) as f32 - 127.0) * 0.6931472
}

/// 1410cb000 / 1410ca700: base-10 exponent approximation, including the
/// underflow branch. Constants were read from the installed executable.
pub fn power10(value: f32) -> f32 {
    if value < -37.0 { return 0.0; }
    let bits = (value * 27_866_352.0 + 1_065_353_216.0) as i64 as u32;
    let mantissa = f32::from_bits((bits & 0x7fffff) + 0x3f800000);
    ((mantissa * 0.32518977 + 0.020805772) * mantissa + 0.65304345) * f32::from_bits(bits & 0xff800000)
}

/// CAkConversionTable::ApplyCurveScaling, 1410cb000. Positive dB scaling
/// uses -log(1-v), while negative scaling uses log(1+v).
fn scale(value: f32, scaling: u8) -> f32 {
    match scaling {
        2 if value >= 1.0 => 4096.0,
        2 if value >= 0.0 => ln(1.0 - value) * -0.4342945 * 20.0,
        2 if value <= -1.0 => -4096.0,
        2 => ln(value + 1.0) * 0.4342945 * 20.0,
        3 => power10(value),
        4 => power10(value * 0.05),
        _ => value,
    }
}

/// AkInterpolation::InterpolateNoCheck, 1410cd3b0. Constant segments are
/// handled by the conversion table without calling the interpolation helper.
pub(crate) fn interpolate(mut t: f32, a: f32, b: f32, mode: u32) -> f32 {
    match mode {
        0 => { t = 1.0 - t; t * t * t * (a - b) + b }
        1 => {
            t *= std::f32::consts::FRAC_PI_2;
            let x = t * t;
            (((0.008306325 - x * 0.00018363654) * x - 0.16664828) * x + 0.9999966) * t * (b - a) + a
        }
        2 => (t - 3.0) * t * 0.5 * (a - b) + a,
        3 => {
            let upper = t > 0.5;
            let angle = if upper { std::f32::consts::PI - t * std::f32::consts::PI } else { t * std::f32::consts::PI };
            let x = angle * angle;
            let v = (((0.0041531627 - x * 0.00009181827) * x - 0.08332414) * x + 0.4999983) * angle;
            (if upper { 1.0 - v } else { v }) * (b - a) + a
        }
        4 => (b - a) * t + a,
        5 => {
            let x = t * std::f32::consts::PI * t * std::f32::consts::PI;
            (((x * 0.000484834 - 0.01961384) * x + 0.2476748) * x + 0.00069670216) * (b - a) + a
        }
        6 => (t + 1.0) * t * 0.5 * (b - a) + a,
        7 => {
            let x = t * std::f32::consts::FRAC_PI_2 * t * std::f32::consts::FRAC_PI_2;
            (((0.04148775 - x * 0.0012712094) * x - 0.49991244) * x + 0.9999933) * (a - b) + b
        }
        8 => t * t * t * (b - a) + a,
        9 => a,
        _ => 0.0,
    }
}

pub fn curve_value(curve: &RtpcCurve, input: f32) -> Option<f32> {
    let first = curve.points.first()?;
    let last = curve.points.last()?;
    let value = if input <= first.input {
        first.output
    } else if input >= last.input {
        last.output
    } else {
        let upper = curve.points.partition_point(|point| point.input <= input);
        let (a, b) = (&curve.points[upper - 1], &curve.points[upper]);
        interpolate((input - a.input) / (b.input - a.input), a.output, b.output, a.interpolation)
    };
    Some(scale(value, curve.scaling))
}

pub struct Mixer {
    pub buses: HashMap<u32, NodeProperties>,
    pub parameters: HashMap<u32, f32>,
    pub volume_threshold_db: f32,
}

impl From<BankMix> for Mixer {
    fn from(bank: BankMix) -> Self {
        Self { buses: bank.buses.into_iter().map(|bus| (bus.id, bus)).collect(), parameters: bank.parameters, volume_threshold_db: bank.volume_threshold_db }
    }
}

impl Mixer {
    fn node_volume(&self, node: &NodeProperties, parameter: Option<(u32, f32)>) -> Result<f32, String> {
        // RTPC parameter IDs differ from static property IDs for makeup gain
        // and output-bus volume in bank version 113.
        let mut values = [node.float(0), node.float(5), node.float(0x21), node.float(0x17)];
        for curve in &node.curves {
            let slot = match curve.property { 0 => 0, 5 => 1, 0x24 => 2, 0x3c => 3, _ => continue };
            if curve.kind != 0 { return Err(format!("node {:#010x} has a non-game-parameter volume curve", node.id)); }
            let input = parameter.filter(|(id, _)| *id == curve.parameter).map(|(_, value)| value)
                .or_else(|| self.parameters.get(&curve.parameter).copied())
                .ok_or_else(|| format!("missing Wwise parameter default {:#010x}", curve.parameter))?;
            let value = curve_value(curve, input).ok_or("empty Wwise volume curve")?;
            match curve.accumulation {
                0 => values[slot] = value,
                1 => values[slot] += value,
                2 => values[slot] *= value,
                _ => return Err("unsupported Wwise volume accumulation".into()),
            }
        }
        Ok(values.into_iter().sum())
    }

    pub fn volume_db(&self, source: &MediaProperties) -> Result<f32, String> {
        self.volume_db_with_parameter(source, None)
    }

    pub fn volume_db_with_parameter(&self, source: &MediaProperties, parameter: Option<(u32, f32)>) -> Result<f32, String> {
        let mut volume = 0.0;
        for node in &source.nodes { volume += self.node_volume(node, parameter)?; }
        let mut bus = source.bus;
        let mut visited = HashSet::new();
        while bus != 0 {
            if !visited.insert(bus) { return Err("cyclic Wwise bus hierarchy".into()); }
            let node = self.buses.get(&bus).ok_or_else(|| format!("missing Wwise output bus {bus:#010x}"))?;
            volume += self.node_volume(node, parameter)?;
            bus = node.parent;
        }
        Ok(volume)
    }

    pub fn gain(&self, source: &MediaProperties, random_db: f32) -> Result<f32, String> {
        self.gain_with_parameter(source, random_db, None)
    }

    pub fn gain_with_parameter(&self, source: &MediaProperties, random_db: f32, parameter: Option<(u32, f32)>) -> Result<f32, String> {
        let volume = self.volume_db_with_parameter(source, parameter)? + random_db;
        Ok(if volume <= self.volume_threshold_db { 0.0 } else { power10(volume * 0.05) })
    }
}
