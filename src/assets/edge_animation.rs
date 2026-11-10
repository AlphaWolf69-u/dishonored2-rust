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
pub(crate) mod residency;
pub use residency::acquire;
pub(crate) use residency::acquire_cancellable;
pub(crate) use residency::{find_existing, force_load, ResourceScope};
pub(crate) use residency::{acquire_record, acquire_record_in_context, find_record, register_lazy_in_context, register_lazy_preload,
    force_load_in_context, mark_preload_persistent, promote_bootstrap, promote_existing_bootstrap,
    release_marker, CreationContext, PurgeMarker, ResourceLease};
pub(crate) use residency::release_level;
pub(crate) use residency::retire_catalog;

/// An immutable compressed resource; poses and playback time belong to actors.
#[derive(Clone, Debug)]
pub struct EdgeAnimation(std::sync::Arc<AnimationData>);
impl std::ops::Deref for EdgeAnimation {
    type Target = AnimationData;
    fn deref(&self) -> &Self::Target { &self.0 }
}

/// sqrt(2)/32767 (DAT_141f69970) and -sqrt(1/2) (DAT_141f69aa0).
const QUAT_SCALE: f32 = 4.315_969e-5;
const QUAT_OFFSET: f32 = -0.707_106_77;

#[doc(hidden)]
#[derive(Debug)]
pub struct AnimationData {
    data: Vec<u8>,
    /// Per-joint blend weights (u16@0x28 bytes at u32@0x30), if present.
    joint_weights: std::ops::Range<usize>,
    /// Trailer `m_flags` (FUN_140240210): bit 0 additive, bits 1..3 remove
    /// root T.x/T.y/T.z, bit 4 remove root rotation.
    pub flags: u16,
    /// Trailer `m_animProperties` after m_flags (FUN_140247190): the weight
    /// group a midnight ANIM_EXPORT_MASK clip plays on.
    pub properties: u32,
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
        let (mut data, size) = Self::parse_header(bytes)?;
        data.data = bytes[..size].to_vec();
        Ok(Self(std::sync::Arc::new(data)))
    }

    fn parse_owned(mut bytes: Vec<u8>) -> Result<Self, String> {
        let (mut data, size) = Self::parse_header(&bytes)?;
        bytes.truncate(size);
        data.data = bytes;
        Ok(Self(std::sync::Arc::new(data)))
    }

    /// The authored joint mask, borrowed from the compressed clip.
    pub fn joint_weights(&self) -> Option<&[u8]> {
        (!self.joint_weights.is_empty()).then(|| &self.data[self.joint_weights.clone()])
    }

    fn retained_bytes(&self) -> usize {
        2 * std::mem::size_of::<usize>() + std::mem::size_of::<AnimationData>()
            + self.data.capacity()
            + self.tables.iter().map(|table| table.capacity() * 2).sum::<usize>()
    }

    fn parse_header(bytes: &[u8]) -> Result<(AnimationData, usize), String> {
        if bytes.get(..4) != Some(b"80AE") {
            return Err("not an Edge animation".into());
        }
        let size = u32_at(bytes, 0x54)? as usize + 0x54;
        let data = bytes.get(..size).ok_or("Edge animation shorter than its header size")?;
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
        let properties = trailer(size + 4)
            .and_then(|length| bytes.get(size + 10 + length..size + 14 + length))
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let weight_count = u16_at(&data, 0x28)? as usize;
        let joint_weights = if weight_count > 0 {
            let at = u32_at(data, 0x30)? as usize;
            let end = at.checked_add(weight_count).ok_or("truncated Edge joint weights")?;
            data.get(at..end).ok_or("truncated Edge joint weights")?;
            at..end
        } else {
            0..0
        };
        Ok((AnimationData {
            joint_weights,
            flags,
            properties,
            frame_rate: f32_at(&data, 0x08)?,
            frame_count: u16_at(&data, 0x10)?,
            frame_set_count: u16_at(&data, 0x12)?,
            counts,
            tables,
            data: Vec::new(),
        }, size))
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
        let frame_time = (time * self.frame_rate).max(0.0);
        self.sample_inner(rest, frame_time, None)
    }

    /// Sample a separately resolved frame and interpolation fraction, as
    /// supplied by 6e0ba0 after applying the graph node's wrap policy.
    pub(crate) fn sample_frame(&self, rest: &[JointRestPose], frame: u16, fraction: f32)
        -> Result<Vec<JointRestPose>, String> {
        self.sample_inner(rest, f32::from(frame), Some((frame, fraction)))
    }

    fn sample_inner(&self, rest: &[JointRestPose], frame_time: f32,
        explicit: Option<(u16, f32)>) -> Result<Vec<JointRestPose>, String> {
        let h = &self.data;
        let mut pose = rest.to_vec();
        let joints = pose.len();
        let joint = |index: u16| -> Result<usize, String> {
            let index = index as usize;
            if index < joints { Ok(index) } else { Err("Edge animation joint outside the skeleton".into()) }
        };
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
        let first = u16_at(h, starts + set * 4)?;
        let local = frame_time - f32::from(first);
        let (mut frame, mut fraction) = if let Some((global, fraction)) = explicit {
            (usize::from(global.saturating_sub(first)), fraction)
        } else {
            let frame = local as usize;
            (frame, local - frame as f32)
        };
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
/// md6anim resource. Call it in the md6's animset order: a later animset's
/// alias replaces an earlier one, as the model def's lookups
/// (FUN_1401c7030 / FUN_1401c7bf0) search the list from its last entry.
#[derive(Clone, Debug)]
pub(crate) struct AnimSetAlias {
    pub animation: String,
    pub force_load: bool,
    pub force_zero_speed: bool,
    pub movement: Option<std::num::NonZeroU32>,
}

/// `246c00`'s persisted animation movement record. Values describe the
/// authored/generated trajectory; they do not modify clip sampling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AnimMovement {
    pub start_speed: f32,
    pub end_speed: f32,
    pub average_speed: f32,
    pub start_yaw: f32,
    pub end_yaw: f32,
    pub average_turn_speed: f32,
    pub strafe_yaw: f32,
    pub slope_angle: f32,
    pub vertical_move_length: f32,
}

/// An `ae_marker` in an AnimSet's animation-event list. The enum values are
/// initialized by 141a08120; time is the authored animation frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AnimMarker { pub kind: u8, pub frame: i16 }

/// Read marker events from the already-tokenized AnimSet declaration. Other
/// event types remain owned by their respective event consumers.
pub(crate) fn visit_animset_markers(tokens: &[&str],
    mut visit: impl FnMut(&str, Vec<AnimMarker>) -> Result<(), String>) -> Result<(), String> {
    fn block_end(tokens: &[&str], open: usize) -> Result<usize, String> {
        if tokens.get(open) != Some(&"{") { return Err("animation event block is absent".into()); }
        let mut depth = 1usize;
        for (index, token) in tokens.iter().enumerate().skip(open + 1) {
            match *token {
                "{" => depth += 1,
                "}" => { depth -= 1; if depth == 0 { return Ok(index); } },
                _ => {}
            }
        }
        Err("unterminated animation event block".into())
    }
    let mut depth = 0usize;
    let mut start = None;
    for (index, token) in tokens.iter().enumerate() {
        if depth <= 1 && *token == "events" && tokens.get(index + 1) == Some(&"{") {
            start = Some(index);
            break;
        }
        match *token { "{" => depth += 1, "}" => depth = depth.saturating_sub(1), _ => {} }
    }
    let Some(start) = start else { return Ok(()) };
    let end = block_end(tokens, start + 1)?;
    let mut at = start + 2;
    while at < end {
        if tokens[at] != "anim" || tokens.get(at + 2) != Some(&"{") {
            return Err("invalid animation event list".into());
        }
        let animation = tokens[at + 1].trim_matches('"');
        let animation_end = block_end(tokens, at + 2)?;
        let mut markers = Vec::new();
        let mut event = at + 3;
        while event < animation_end {
            if tokens[event] != "event" || tokens.get(event + 2) != Some(&"{") {
                return Err("invalid animation event".into());
            }
            let event_end = block_end(tokens, event + 2)?;
            if tokens[event + 1].trim_matches('"') == "ae_marker" {
                let body = &tokens[event + 3..event_end];
                let frame = body.iter().position(|token| *token == "frame")
                    .and_then(|index| body.get(index + 1)).ok_or("animation marker frame is absent")?
                    .parse::<i16>().map_err(|_| "animation marker frame is invalid")?;
                let name = body.iter().position(|token| *token == "arkMidnightAnimMarker::Name_t")
                    .and_then(|index| body.get(index + 1)).ok_or("animation marker kind is absent")?.trim_matches('"');
                let kind = match name { "EMAM_IN" => 0, "EMAM_APEX" => 1, "EMAM_ACCENT" => 2,
                    "EMAM_LOOP" => 3, "EMAM_LOOPEND" => 4, "EMAM_OUT" => 5,
                    "EMAM_INVALID" | "EMAM_MAX" => 6, _ => return Err(format!("unknown animation marker {name}")) };
                markers.push(AnimMarker { kind, frame });
            }
            event = event_end + 1;
        }
        if !markers.is_empty() { visit(animation, markers)?; }
        at = animation_end + 1;
    }
    Ok(())
}

/// Visit aliases in declaration order. Native 257390 loads a forced clip
/// before its caller inserts that alias, including when a name is repeated.
pub(crate) fn visit_animset_aliases(
    source: &str,
    visit: impl FnMut(String, AnimSetAlias) -> Result<(), String>,
) -> Result<(), String> {
    let tokens = crate::entities::declaration_lexemes(source)?;
    visit_animset_alias_tokens(&tokens, visit)
}

pub(crate) fn visit_animset_alias_tokens(
    tokens: &[&str],
    mut visit: impl FnMut(String, AnimSetAlias) -> Result<(), String>,
) -> Result<(), String> {
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "alias" && tokens.get(i + 1) == Some(&"{") {
            let (mut name, mut anim) = (None, None);
            let (mut force_load, mut force_zero_speed) = (false, false);
            let mut k = i + 2;
            let mut depth = 0;
            let mut flags_depth = None;
            while k < tokens.len() && !(depth == 0 && tokens[k] == "}") {
                match tokens[k] {
                    "flags" if depth == 0 && tokens.get(k + 1) == Some(&"{") => flags_depth = Some(depth + 1),
                    "{" => depth += 1,
                    "}" => { if flags_depth == Some(depth) { flags_depth = None; } depth -= 1; },
                    "name" if depth == 0 => name = tokens.get(k + 1).map(|t| t.trim_matches('"').to_owned()),
                    "anim" if depth == 0 => anim = tokens.get(k + 1).map(|t| t.trim_matches('"').to_owned()),
                    flag if flags_depth == Some(depth) && flag.eq_ignore_ascii_case("forceLoad") => force_load = true,
                    flag if flags_depth == Some(depth) && flag.eq_ignore_ascii_case("forceZeroSpeed") => force_zero_speed = true,
                    _ => {}
                }
                k += 1;
            }
            if let (Some(name), Some(animation)) = (name, anim) {
                visit(name, AnimSetAlias { animation, force_load, force_zero_speed, movement: None })?;
            }
            i = k;
        }
        i += 1;
    }
    Ok(())
}

/// `2462b0` walks generated alias references in source order. Required
/// movement fields use the same zero-initialized optional fields as 246c00.
pub(crate) fn visit_animset_movement(
    tokens: &[&str],
    mut visit: impl FnMut(&str, AnimMovement) -> Result<(), String>,
) -> Result<(), String> {
    let Some(start) = tokens.windows(2).position(|pair| pair == ["animGeneratedData", "{"]) else { return Ok(()); };
    let mut at = start + 2;
    while at < tokens.len() && tokens[at] != "}" {
        if tokens[at] != "aliasRef" || tokens.get(at + 2) != Some(&"{") {
            return Err("invalid animGeneratedData alias reference".into());
        }
        let name = tokens.get(at + 1).ok_or("missing generated alias name")?.trim_matches('"');
        let mut end = at + 3;
        let mut depth = 1usize;
        while end < tokens.len() && depth != 0 {
            match tokens[end] { "{" => depth += 1, "}" => depth -= 1, _ => {} }
            end += 1;
        }
        if depth != 0 { return Err(format!("unterminated generated alias {name}")); }
        let body = &tokens[at + 3..end - 1];
        let marker = body.windows(2).position(|pair| pair == ["animMovementProp", "{"])
            .ok_or_else(|| format!("generated alias {name} has no movement record"))?;
        let rest = &body[marker + 2..];
        let close = rest.iter().position(|token| *token == "}")
            .ok_or_else(|| format!("unterminated movement record for {name}"))?;
        let fields = &rest[..close];
        let value = |key: &str, required: bool| -> Result<f32, String> {
            match fields.iter().position(|field| *field == key) {
                Some(index) => fields.get(index + 1).ok_or_else(|| format!("missing {key} for {name}"))?
                    .parse().map_err(|_| format!("invalid {key} for {name}")),
                None if !required => Ok(0.0),
                None => Err(format!("missing {key} for {name}")),
            }
        };
        visit(name, AnimMovement {
            start_speed: value("startSpeed", true)?, end_speed: value("endSpeed", true)?,
            average_speed: value("averageSpeed", true)?, start_yaw: value("startYaw", true)?,
            end_yaw: value("endYaw", true)?, average_turn_speed: value("averageTurnSpeed", true)?,
            strafe_yaw: value("strafeYaw", false)?, slope_angle: value("slopeAngle", true)?,
            vertical_move_length: value("verticalMoveLength", false)?,
        })?;
        at = end;
    }
    if at == tokens.len() { return Err("unterminated animGeneratedData".into()); }
    Ok(())
}

pub fn animset_aliases(source: &str, aliases: &mut std::collections::HashMap<String, String>) -> Result<(), String> {
    visit_animset_aliases(source, |name, alias| { aliases.insert(name, alias.animation); Ok(()) })
}

/// The compiled resource of an md6anim path.
pub fn resource_path(anim: &str) -> Option<String> {
    anim.strip_suffix(".md6anim").map(|stem| format!("generated/anim/{stem}.edgeanim"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_skip_nested_blocks_and_later_animsets_win() {
        let mut aliases = std::collections::HashMap::new();
        let base = r#"{ alias { name "lean" flags { forceLoad } anim "base/lean.md6anim" } group 0
            alias { name "run" anim "emily/run.md6anim" } }"#;
        animset_aliases(base, &mut aliases).unwrap();
        animset_aliases(r#"{ alias { name "run" anim "corvo/run.md6anim" } }"#, &mut aliases).unwrap();
        assert_eq!(aliases.get("lean").map(String::as_str), Some("base/lean.md6anim"));
        assert_eq!(aliases.get("run").map(String::as_str), Some("corvo/run.md6anim"));
    }

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
