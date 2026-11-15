//! Marker movie slots 538ec0/539ae0 and projection/clamping 6763e0.
//! Host: normalized projection arrives from the existing camera and entity poses.
use super::*;
use crate::app::objective_markers::{Key, Marker};
use crate::flash::avm2::value::Value;
use crate::gameplay::options::decl::{atom, float, scope};
struct Slot {
    key: Key,
    kind: u8,
    marked: bool,
    root: Value,
    locator: Value,
    last: Option<[f64; 6]>,
}
#[derive(Clone, Copy)]
struct Style {
    scale: [f32; 4],
    alpha: [f32; 4],
    offset: [f32; 2],
}
pub(super) struct State {
    slots: Vec<Option<Slot>>,
    styles: [Style; 8],
    clamp: [f32; 2],
    mode: u8,
}
impl State {
    pub(super) fn from_fields(fields: &[(String, crate::entities::Value)]) -> Result<Self, String> {
        let area = scope(&fields, "m_clampingArea").ok_or("objective clamping area missing")?;
        let setup = scope(&fields, "m_markerSetup").ok_or("objective marker setup missing")?;
        let mut styles = [Style {
            scale: [1., 50., 50., 23.],
            alpha: [0., 0., 1., 1.],
            offset: [0., 0.],
        }; 8];
        for (index, name) in [
            "MARKER_OBJECTIVE",
            "MARKER_OBJECTIVE_OPTIONAL",
            "MARKER_OBJECTIVE_LETHAL",
            "MARKER_OBJECTIVE_LETHAL_OPTIONAL",
            "MARKER_OBJECTIVE_NON_LETHAL",
            "MARKER_OBJECTIVE_NON_LETHAL_OPTIONAL",
            "MARKER_OBJECTIVE_MIXED",
            "MARKER_OBJECTIVE_MIXED_OPTIONAL",
        ]
        .iter()
        .enumerate()
        {
            let fields = scope(setup, &format!("enumItem[{name}]"))
                .ok_or_else(|| format!("missing objective marker style {name}"))?;
            let scale = scope(fields, "m_markerScale").ok_or("objective marker scale missing")?;
            let alpha = scope(fields, "m_markerAlpha").ok_or("objective marker alpha missing")?;
            let min = scope(alpha, "m_alphaAtMinDistance").unwrap_or(&[]);
            let max = scope(alpha, "m_alphaAtMaxDistance").unwrap_or(&[]);
            let position = scope(fields, "m_markerPosition")
                .and_then(|fields| scope(fields, "m_info"))
                .and_then(|fields| scope(fields, "item[0]"))
                .unwrap_or(&[]);
            let offset = scope(position, "m_screenOffset")
                .and_then(|fields| scope(fields, "m_offset"))
                .unwrap_or(&[]);
            styles[index] = Style {
                scale: [
                    float(scale, "m_minDistanceForScale", 1.),
                    float(scale, "m_maxDistanceForScale", 50.),
                    float(scale, "m_scaleAtMinDistance", 50.),
                    float(scale, "m_scaleAtMaxDistance", 23.),
                ],
                alpha: [
                    float(min, "m_distance", 0.),
                    float(max, "m_distance", 50.),
                    float(min, "m_alphaValue", 1.),
                    float(max, "m_alphaValue", 1.),
                ],
                offset: [float(offset, "x", 0.), float(offset, "y", 0.)],
            };
        }
        let mode = match atom(&fields, "m_clampingMode") {
            Some("MARKER_CLAMPING_RECTANGLE") => 0,
            Some("MARKER_CLAMPING_ELLIPSE") => 1,
            Some("MARKER_CLAMPING_NONE") => 2,
            other => return Err(format!("unsupported marker clamping {other:?}")),
        };
        Ok(Self {
            slots: Vec::new(),
            styles,
            clamp: [float(area, "x", 0.), float(area, "y", 0.)],
            mode,
        })
    }
}
fn interpolate(values: [f32; 4], distance: f32) -> f32 {
    let [min, max, a, b] = values;
    let t = if min == max {
        0.
    } else {
        (distance.clamp(min, max) - min) / (max - min)
    };
    a + (b - a) * t
}
impl HudMovie {
    pub(crate) fn markers(&mut self, markers: &[Marker]) -> Result<(), String> {
        for index in 0..self.markers.slots.len() {
            if self.markers.slots[index]
                .as_ref()
                .is_some_and(|slot| !markers.iter().any(|marker| marker.key == slot.key))
            {
                self.player
                    .call_path_method(
                        "_markersManager",
                        "_C_RemoveMarker",
                        &[(index as u32).into()],
                    )
                    .map_err(|error| self.player.describe(&error))?;
                self.markers.slots[index] = None;
            }
        }
        let (width, height) = self.player.state.borrow().stage_size;
        for marker in markers {
            let existing = self
                .markers
                .slots
                .iter()
                .position(|slot| slot.as_ref().is_some_and(|slot| slot.key == marker.key));
            let index = existing.unwrap_or_else(|| {
                self.markers
                    .slots
                    .iter()
                    .position(Option::is_none)
                    .unwrap_or(self.markers.slots.len())
            });
            if existing.is_some_and(|index| {
                self.markers.slots[index]
                    .as_ref()
                    .is_some_and(|slot| slot.kind != marker.kind || slot.marked != marker.marked)
            }) {
                self.player
                    .call_path_method(
                        "_markersManager",
                        "_C_RemoveMarker",
                        &[(index as u32).into()],
                    )
                    .map_err(|error| self.player.describe(&error))?;
                self.markers.slots[index] = None;
            }
            if self.markers.slots.get(index).is_none_or(Option::is_none) {
                self.player
                    .call_path_method(
                        "_markersManager",
                        "_C_AddMarker",
                        &[
                            (index as u32).into(),
                            (marker.kind as u32).into(),
                            marker.marked.into(),
                        ],
                    )
                    .map_err(|error| self.player.describe(&error))?;
                let path = format!("_markersManager._list[{index}]");
                let slot = Slot {
                    key: marker.key,
                    kind: marker.kind,
                    marked: marker.marked,
                    root: self
                        .player
                        .get_value(&path)
                        .map_err(|error| self.player.describe(&error))?,
                    locator: self
                        .player
                        .get_value(&format!("{path}.locator"))
                        .map_err(|error| self.player.describe(&error))?,
                    last: None,
                };
                if index == self.markers.slots.len() {
                    self.markers.slots.push(Some(slot));
                } else {
                    self.markers.slots[index] = Some(slot);
                }
            }
            let style = self.markers.styles[marker.kind as usize];
            let mut x = (marker.ndc[0] as f64 + 1.) * width * 0.5;
            let mut y = (1. - marker.ndc[1] as f64) * height * 0.5;
            if marker.facing >= 0. {
                x += style.offset[0] as f64 * height / 1080.;
                y += style.offset[1] as f64 * height / 1080.;
            }
            let mut clamped = false;
            if self.markers.mode != 2 {
                if marker.facing < 0. {
                    x = width - x;
                    y = height - y - y * marker.facing as f64;
                }
                let dx = (x - width * 0.5) / width;
                let dy = (y - height * 0.5) / height;
                let hx = self.markers.clamp[0] as f64 / 100.;
                let hy = self.markers.clamp[1] as f64 / 100.;
                if self.markers.mode == 0 {
                    if dx.abs() > hx || dy.abs() > hy || marker.facing < 0. {
                        // 6763e0 uses FLT_MIN, with a unit ratio on an
                        // exactly axial/zero direction (including behind view).
                        let minimum = f64::from(f32::MIN_POSITIVE);
                        let ratio = if dy.abs() >= minimum {
                            dx / dy.abs()
                        } else {
                            1.
                        };
                        if ratio.abs() >= 1. {
                            let vertical = if dx.abs() >= minimum {
                                dy / dx.abs()
                            } else {
                                1.
                            };
                            x = width * (0.5 + if dx >= 0. { hx } else { -hx });
                            y = height * (0.5 + hy * vertical);
                        } else {
                            x = width * (0.5 + hx * ratio);
                            y = height * (0.5 + if dy >= 0. { hy } else { -hy });
                        }
                        clamped = true;
                    }
                } else {
                    let length = ((dx / hx).powi(2) + (dy / hy).powi(2)).sqrt();
                    if length > 1. || marker.facing < 0. {
                        if length != 0. {
                            x = width * (0.5 + dx / length);
                            y = height * (0.5 + dy / length);
                        } else {
                            x = width * 0.5;
                            y = height * (0.5 + hy);
                        }
                        clamped = true;
                    }
                }
            }
            let scale = interpolate(style.scale, marker.distance) as f64;
            let alpha = interpolate(style.alpha, marker.distance).clamp(0., 1.) as f64;
            let rotation = if clamped {
                (y - height * 0.5).atan2(x - width * 0.5).to_degrees() - 90.
            } else {
                0.
            };
            let next = [
                x - width * 0.5,
                y - height * 0.5,
                scale,
                alpha,
                rotation,
                if clamped { 1. } else { 0. },
            ];
            let slot = self.markers.slots[index].as_mut().unwrap();
            if slot.last == Some(next) {
                continue;
            }
            if slot.last.is_none_or(|last| last[5] != next[5]) {
                self.player
                    .call_path_method(
                        "_markersManager",
                        "_C_LockMarker",
                        &[(index as u32).into(), clamped.into()],
                    )
                    .map_err(|error| self.player.describe(&error))?;
            }
            for (field, value) in [
                ("x", next[0]),
                ("y", next[1]),
                ("scaleX", scale),
                ("scaleY", scale),
                ("alpha", alpha),
            ] {
                self.player
                    .set_member(&slot.root, field, value.into())
                    .map_err(|error| self.player.describe(&error))?;
            }
            self.player
                .set_member(&slot.locator, "rotation", rotation.into())
                .map_err(|error| self.player.describe(&error))?;
            slot.last = Some(next);
            self.last = None;
        }
        Ok(())
    }
}
