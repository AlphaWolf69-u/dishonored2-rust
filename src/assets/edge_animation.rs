//! Installed Edge animations (`.edgeanim`, tag "80AE"): the load size and
//! duration (FUN_14024adf0, FUN_140240150) and single-pose sampling
//! (FUN_1412e2c30 frame-set lookup, FUN_1412e6a70 channel dispatch,
//! FUN_1412e8940/FUN_1412f4ba0 constant and FUN_1412e8f10/FUN_1412f3260
//! animated rotations/translations/scales).
//!
//! Only the default compression paths are decoded: files with the custom
//! rotation/translation flags (bits 0, 1, 3, 4) or animated user channels
//! are rejected. The slerp uses an exact reciprocal square root where the
//! game's SSE path uses unrefined RSQRTPS (differences up to ~2e-4).
use crate::edge_skeleton::JointRestPose;

/// sqrt(2)/32767 (DAT_141f69970) and -sqrt(1/2) (DAT_141f69aa0).
const QUAT_SCALE: f32 = 4.315_969e-5;
const QUAT_OFFSET: f32 = -0.707_106_77;

#[derive(Clone, Debug)]
pub struct EdgeAnimation {
    data: Vec<u8>,
    /// Per-joint blend weights (u16@0x28 bytes at u32@0x30), if present.
    pub joint_weights: Option<Vec<u8>>,
    /// Trailer `m_flags` (FUN_140240210): bit 0 additive, bits 1..3 remove
    /// root T.x/T.y/T.z, bit 4 remove root rotation.
    pub flags: u16,
    pub frame_rate: f32,
    pub frame_count: u16,
    frame_set_count: u16,
    counts: [u16; 8],
    tables: [Vec<u16>; 8],
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, String> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| "truncated Edge animation".to_owned())
}

fn u32_at(d: &[u8], o: usize) -> Result<u32, String> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).ok_or_else(|| "truncated Edge animation".to_owned())
}

fn f32_at(d: &[u8], o: usize) -> Result<f32, String> {
    u32_at(d, o).map(f32::from_bits)
}

/// A self-relative offset field; 0 means absent.
fn relative(d: &[u8], o: usize) -> Result<Option<usize>, String> {
    let v = u32_at(d, o)? as usize;
    Ok((v != 0).then_some(o + v))
}

fn vec3(d: &[u8], o: usize) -> Result<[f32; 3], String> {
    Ok([f32_at(d, o)?, f32_at(d, o + 4)?, f32_at(d, o + 8)?])
}

/// 48-bit "smallest three": big-endian bits 47..32, 31..17, 16..2 hold the
/// kept components, bits 1..0 the index (x, y, z, w) of the dropped one.
fn quat48(d: &[u8], o: usize) -> Result<[f32; 4], String> {
    let bytes = d.get(o..o + 6).ok_or("truncated Edge rotation")?;
    let v = bytes.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64);
    let a = ((v >> 32) & 0xffff) as f32 * QUAT_SCALE + QUAT_OFFSET;
    let b = ((v >> 17) & 0x7fff) as f32 * QUAT_SCALE + QUAT_OFFSET;
    let c = ((v >> 2) & 0x7fff) as f32 * QUAT_SCALE + QUAT_OFFSET;
    let m = (1.0 - a * a - b * b - c * c).max(0.0).sqrt();
    let q = [a, b, c];
    let mut out = [0.0; 4];
    let dropped = (v & 3) as usize;
    let mut k = 0;
    for (i, slot) in out.iter_mut().enumerate() {
        if i == dropped {
            *slot = m;
        } else {
            *slot = q[k];
            k += 1;
        }
    }
    Ok(out)
}

fn sin_poly(a: f32) -> f32 {
    let a2 = a * a;
    ((((a2 * 2.752_6e-6 - 1.984_090_1e-4) * a2 + 8.333_332e-3) * a2 - 0.166_666_67) * a2 + 1.0) * a
}

/// FUN_1412e8f10's polynomial slerp: lerp above |dot| 0.999, no renormalise.
pub fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    let x = dot.abs();
    let (mut wa, wb) = if x > 0.999 {
        (1.0 - t, t)
    } else {
        let theta = (1.0 - x).sqrt() * (1.570_796_3 - (0.214_598_8 - (0.088_978_99 - x * 0.050_174_303 * x) * x) * x);
        let s = sin_poly(theta);
        (sin_poly((1.0 - t) * theta) / s, sin_poly(t * theta) / s)
    };
    wa *= sign;
    std::array::from_fn(|i| wa * a[i] + wb * b[i])
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
}

impl EdgeAnimation {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..4) != Some(b"80AE") {
            return Err("not an Edge animation".into());
        }
        let size = u32_at(bytes, 0x54)? as usize + 0x54;
        let data = bytes.get(..size).ok_or("Edge animation shorter than its header size")?.to_vec();
        let flags = u32_at(&data, 0x2c)?;
        if flags & 0x1b != 0 {
            return Err(format!("Edge animation compression flags {flags:#x} are not decoded"));
        }
        let mut counts = [0u16; 8];
        for (i, count) in counts.iter_mut().enumerate() {
            *count = u16_at(&data, 0x16 + 2 * i)?;
        }
        if counts[7] != 0 {
            return Err("animated Edge user channels are not decoded".into());
        }
        // Joint index tables from +0x60: constant R (padded to 8), then the
        // others padded to 4.
        let mut at = 0x60;
        let tables = std::array::from_fn(|i| {
            let count = counts[i] as usize;
            let pad = if i == 0 { 8 } else { 4 };
            let table: Vec<u16> = (0..count).map(|k| u16_at(&data, at + 2 * k).unwrap_or(0)).collect();
            at += ((count + pad - 1) & !(pad - 1)) * 2;
            table
        });
        // Trailer after the blob: u32 9, length-prefixed md6skl path, u16 m_flags.
        let trailer = |o: usize| bytes.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
        let flags = trailer(size + 4)
            .and_then(|length| bytes.get(size + 8 + length..size + 10 + length))
            .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        let weight_count = u16_at(&data, 0x28)? as usize;
        let joint_weights = if weight_count > 0 {
            let at = u32_at(&data, 0x30)? as usize;
            Some(data.get(at..at + weight_count).ok_or("truncated Edge joint weights")?.to_vec())
        } else {
            None
        };
        Ok(Self {
            joint_weights,
            flags,
            frame_rate: f32_at(&data, 0x08)?,
            frame_count: u16_at(&data, 0x10)?,
            frame_set_count: u16_at(&data, 0x12)?,
            counts,
            tables,
            data,
        })
    }

    /// FUN_140240150: `(frames - 1) / ceil(rate)`, 0 for a still pose.
    pub fn duration(&self) -> f32 {
        let rate = self.frame_rate.ceil();
        if rate != 0.0 && self.frame_count > 1 {
            (self.frame_count - 1) as f32 / rate
        } else {
            0.0
        }
    }

    /// Local joint poses at `time` seconds over the skeleton's rest pose.
    pub fn sample(&self, rest: &[JointRestPose], time: f32) -> Result<Vec<JointRestPose>, String> {
        let h = &self.data;
        let mut pose = rest.to_vec();
        let joints = pose.len();
        let joint = |index: u16| -> Result<usize, String> {
            let index = index as usize;
            if index < joints { Ok(index) } else { Err("Edge animation joint outside the skeleton".into()) }
        };
        let frame_time = (time * self.frame_rate).max(0.0);
        let starts = relative(h, 0x3c)?.ok_or("Edge animation has no frame-set starts")?;
        let (mut lo, mut hi) = (0usize, self.frame_set_count.saturating_sub(1) as usize);
        while lo + 1 < hi {
            let mid = (hi + lo) >> 1;
            if (frame_time as i32) < u16_at(h, starts + mid * 4)? as i32 {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let set = lo;
        let n = u16_at(h, starts + set * 4 + 2)? as usize;
        let local = frame_time - u16_at(h, starts + set * 4)? as f32;
        let mut frame = local as usize;
        let mut fraction = local - frame as f32;
        if n < frame {
            frame = n;
            fraction = 1.0;
        }
        let constant = |field: usize| relative(h, field);
        if let Some(at) = constant(0x40)? {
            for (i, &j) in self.tables[0].iter().enumerate() {
                pose[joint(j)?].rotation = quat48(h, at + 6 * i)?;
            }
        }
        if let Some(at) = constant(0x44)? {
            for (i, &j) in self.tables[1].iter().enumerate() {
                pose[joint(j)?].translation = vec3(h, at + 12 * i)?;
            }
        }
        if let Some(at) = constant(0x48)? {
            for (i, &j) in self.tables[2].iter().enumerate() {
                pose[joint(j)?].scale = vec3(h, at + 12 * i)?;
            }
        }
        if self.frame_set_count <= 1 {
            return Ok(pose);
        }
        let sets = relative(h, 0x38)?.ok_or("Edge animation has no frame sets")?;
        let p = u32_at(h, sets + set * 8 + 4)? as usize;
        let size = |o: usize, i: usize| -> Result<usize, String> { Ok(u32_at(h, o + 4 * i)? as usize) };
        let initial_r = p + 0x20;
        let initial_t = initial_r + size(p, 0)?;
        let initial_s = initial_t + size(p, 1)?;
        let initial_u = initial_s + size(p, 2)?;
        let bits = initial_u + size(p, 3)?;
        let animated = (self.counts[4] + self.counts[5] + self.counts[6] + self.counts[7]) as usize;
        let key_r = bits + ((animated * n + 7) >> 3);
        let key_t = key_r + size(p, 4)?;
        let key_s = key_t + size(p, 5)?;
        let key_u = (key_s + size(p, 6)? + 3) & !3;
        let q = (key_u + size(p, 7)? + 15) & !15;
        let final_r = q + 0x20;
        let final_t = final_r + size(q, 0)?;
        let final_s = final_t + size(q, 1)?;
        let bit = |index: usize| -> Result<bool, String> {
            let byte = *h.get(bits + index / 8).ok_or("truncated Edge key bits")?;
            Ok(byte & (0x80 >> (index % 8)) != 0)
        };
        // (keys before `frame`, keys in the set, previous key frame, next key frame).
        let keys = |channel: usize| -> Result<(usize, usize, usize, usize), String> {
            let mut before = 0;
            let mut count = 0;
            let mut previous = 0;
            let mut next = n + 1;
            for k in 0..n {
                if bit(channel * n + k)? {
                    count += 1;
                    if k < frame {
                        before += 1;
                        previous = k + 1;
                    } else if next == n + 1 {
                        next = k + 1;
                    }
                }
            }
            Ok((before, count, previous, next))
        };
        let alpha = |previous: usize, next: usize| (frame as f32 + fraction - previous as f32) / (next - previous) as f32;
        let mut running = 0;
        for (i, &j) in self.tables[4].iter().enumerate() {
            let (before, count, previous, next) = keys(i)?;
            let a = if before == 0 { quat48(h, initial_r + 6 * i)? } else { quat48(h, key_r + 6 * (running + before - 1))? };
            let b = if before == count { quat48(h, final_r + 6 * i)? } else { quat48(h, key_r + 6 * (running + before))? };
            pose[joint(j)?].rotation = slerp(a, b, alpha(previous, next));
            running += count;
        }
        running = 0;
        let base = self.counts[4] as usize;
        for (i, &j) in self.tables[5].iter().enumerate() {
            let (before, count, previous, next) = keys(base + i)?;
            let a = if before == 0 { vec3(h, initial_t + 12 * i)? } else { vec3(h, key_t + 12 * (running + before - 1))? };
            let b = if before == count { vec3(h, final_t + 12 * i)? } else { vec3(h, key_t + 12 * (running + before))? };
            pose[joint(j)?].translation = lerp3(a, b, alpha(previous, next));
            running += count;
        }
        running = 0;
        let base = base + self.counts[5] as usize;
        for (i, &j) in self.tables[6].iter().enumerate() {
            let (before, count, previous, next) = keys(base + i)?;
            let a = if before == 0 { vec3(h, initial_s + 12 * i)? } else { vec3(h, key_s + 12 * (running + before - 1))? };
            let b = if before == count { vec3(h, final_s + 12 * i)? } else { vec3(h, key_s + 12 * (running + before))? };
            pose[joint(j)?].scale = lerp3(a, b, alpha(previous, next));
            running += count;
        }
        Ok(pose)
    }
}

/// `animsets` alias tables (`alias { name ... anim ... }`): alias name →
/// md6anim resource, first definition in animset order.
pub fn animset_aliases(source: &str, aliases: &mut std::collections::HashMap<String, String>) -> Result<(), String> {
    let tokens = crate::entities::declaration_lexemes(source)?;
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "alias" && tokens.get(i + 1) == Some(&"{") {
            let mut name = None;
            let mut anim = None;
            let mut k = i + 2;
            while k < tokens.len() && tokens[k] != "}" {
                match tokens[k] {
                    "name" => name = tokens.get(k + 1).map(|t| t.trim_matches('"').to_owned()),
                    "anim" => anim = tokens.get(k + 1).map(|t| t.trim_matches('"').to_owned()),
                    _ => {}
                }
                k += 1;
            }
            if let (Some(name), Some(anim)) = (name, anim) {
                aliases.entry(name).or_insert(anim);
            }
            i = k;
        }
        i += 1;
    }
    Ok(())
}

/// The compiled resource of an md6anim path.
pub fn resource_path(anim: &str) -> Option<String> {
    anim.strip_suffix(".md6anim").map(|stem| format!("generated/anim/{stem}.edgeanim"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smallest_three_restores_the_dropped_component() {
        // a = b = c = 0 after offset: raw 16384 ≈ 0.7071/scale; dropped w.
        let raw = |v: f32| ((v - QUAT_OFFSET) / QUAT_SCALE).round() as u64;
        let packed = (raw(0.0) << 32) | (raw(0.0) << 17) | (raw(0.0) << 2) | 3;
        let bytes: Vec<u8> = (0..6).rev().map(|k| (packed >> (8 * k)) as u8).collect();
        let q = quat48(&bytes, 0).unwrap();
        assert!(q[0].abs() < 1e-4 && q[1].abs() < 1e-4 && q[2].abs() < 1e-4, "{q:?}");
        assert!((q[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn slerp_endpoints_and_midpoint() {
        let a = [0.0, 0.0, 0.0, 1.0];
        let half = std::f32::consts::FRAC_PI_4;
        let b = [0.0, 0.0, half.sin(), half.cos()];
        let start = slerp(a, b, 0.0);
        let end = slerp(a, b, 1.0);
        assert!(start.iter().zip(&a).all(|(x, y)| (x - y).abs() < 1e-5));
        assert!(end.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5));
        let mid = slerp(a, b, 0.5);
        let quarter = std::f32::consts::FRAC_PI_8;
        assert!((mid[2] - quarter.sin()).abs() < 1e-4 && (mid[3] - quarter.cos()).abs() < 1e-4, "{mid:?}");
    }
}
