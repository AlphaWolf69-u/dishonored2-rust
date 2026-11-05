//! Bounded action history for an F8-only CPU projection of published arm meshes.
use crate::{bmd6, edge_skeleton};
use bevy::prelude::*;
use std::collections::VecDeque;

const MAX_FRAMES: usize = 64;
type Palette = Vec<[[f32; 4]; 3]>;

struct Frame {
    clip_from_world: Mat4,
    palette: Palette,
}

#[derive(Default)]
pub(super) struct ActionCapture {
    models: [VecDeque<Frame>; 3],
    dropped: [u32; 3],
    missing_camera: u32,
}

impl ActionCapture {
    pub fn push(&mut self, model: usize, clip_from_world: Option<Mat4>, palette: &[[[f32; 4]; 3]]) {
        let Some(frames) = self.models.get_mut(model) else { return };
        let Some(clip_from_world) = clip_from_world else {
            self.missing_camera = self.missing_camera.saturating_add(1);
            return;
        };
        if frames.len() == MAX_FRAMES {
            let mut frame = frames.pop_front().expect("full capture ring");
            self.dropped[model] = self.dropped[model].saturating_add(1);
            frame.clip_from_world = clip_from_world;
            frame.palette.clear();
            frame.palette.extend_from_slice(palette);
            frames.push_back(frame);
        } else {
            frames.push_back(Frame { clip_from_world, palette: palette.to_vec() });
        }
    }

    /// Conservative homogeneous triangle clipping. It does not sample depth,
    /// alpha, face culling, or GPU submission, and cannot establish occlusion.
    pub fn describe(&self, model: usize, info: &bmd6::ModelInfo, surfaces: &[usize], depth_hack: f32) -> String {
        let Some(frames) = self.models.get(model) else { return String::new() };
        let mut no_geometry = 0u32;
        let mut potential = 0u32;
        let mut near_only = 0u32;
        let mut outside = 0u32;
        let mut failed = 0u32;
        for frame in frames {
            match project(frame, info, surfaces, depth_hack) {
                Ok((0, _, _)) => no_geometry += 1,
                Ok((_, true, _)) => potential += 1,
                Ok((_, false, true)) => near_only += 1,
                Ok(_) => outside += 1,
                Err(_) => failed += 1,
            }
        }
        format!("samples={} dropped={} no_geometry={} possibly_intersects_frustum={} some_near_clipped_none_intersect={} outside_frustum={} projection_errors={} missing_camera={} depth_hack={} (conservative, pre-jitter camera; no depth/alpha/occlusion check)",
            frames.len(), self.dropped[model], no_geometry, potential, near_only, outside, failed, self.missing_camera, depth_hack)
    }
}

fn project(frame: &Frame, info: &bmd6::ModelInfo, surfaces: &[usize], hack: f32) -> Result<(usize, bool, bool), String> {
    let mut triangles = 0;
    let mut potentially_visible = false;
    let mut near_candidate = false;
    for &index in surfaces {
        let surface = info.surfaces.get(index).ok_or("missing arm surface")?;
        if surface.positions.len() != surface.skin_indices.len() || surface.positions.len() != surface.skin_weights.len() {
            return Err("arm skin stream mismatch".into());
        }
        let mut projected = Vec::with_capacity(surface.positions.len());
        for (i, p) in surface.positions.iter().enumerate() {
            let m = edge_skeleton::vertex_skin_matrix(&frame.palette, surface.matrix_offset,
                surface.skin_indices[i], surface.skin_weights[i], surface.skin_influences)?;
            let v = Vec4::new(p[0], p[1], p[2], 1.0);
            let game = Vec3::new(Vec4::from_array(m[0]).dot(v), Vec4::from_array(m[1]).dot(v), Vec4::from_array(m[2]).dot(v));
            let mut clip = frame.clip_from_world * Vec4::new(game.x, game.z, -game.y, 1.0);
            clip.z *= hack;
            projected.push(clip);
        }
        for tri in surface.indices.chunks_exact(3) {
            let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            let (Some(a), Some(b), Some(c)) = (projected.get(a), projected.get(b), projected.get(c)) else {
                return Err("arm triangle index outside vertex stream".into());
            };
            triangles += 1;
            let mask = |p: &Vec4| -> u8 {
                u8::from(p.x < -p.w) | (u8::from(p.x > p.w) << 1)
                    | (u8::from(p.y < -p.w) << 2) | (u8::from(p.y > p.w) << 3)
                    | (u8::from(p.z < 0.0) << 4) | (u8::from(p.z > p.w) << 5)
            };
            let rejected = mask(a) & mask(b) & mask(c);
            if rejected == 0 { potentially_visible = true; }
            if rejected == 0x20 { near_candidate = true; }
        }
    }
    Ok((triangles, potentially_visible, near_candidate))
}
