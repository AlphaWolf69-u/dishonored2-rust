//! Camera-effect settings and material selection from 313e70 / 315c20.
//! The view owner supplies the seed: fields not written by the updater (such
//! as the optical centre and physical-lens state) retain their existing bits.
pub const PARAMETER_NAMES: [&str; 33] = [
    "env/dof/settings/bokeh_type",
    "env/dof/settings/bokeh_size",
    "env/dof/settings/enable_autofocus",
    "env/dof/settings/autofocus_time",
    "env/dof/settings/debug_focal_plane",
    "env/dof/settings/use_asymetric_lens",
    "env/dof/settings/coc_focal_distance",
    "env/dof/settings/coc_focus_range_near",
    "env/dof/settings/coc_focus_range_near_sharp",
    "env/dof/settings/coc_focus_range_far_sharp",
    "env/dof/settings/coc_focus_range_far",
    "env/dof/settings/coc_focus_bias",
    "env/dof/settings/fringing_chroma",
    "env/dof/settings/fringing_threshold",
    "env/dof/settings/fringing_gain",
    "env/dof/settings/distortion_coef",
    "env/dof/settings/distortion_cubic_coef",
    "env/dof/settings/sharpen_coef",
    "env/dof/settings/vignette_out",
    "env/dof/settings/vignette_in",
    "env/dof/settings/vignette_fade",
    "env/dof/settings/vignette_fstop",
    "env/dof/settings/use_fringing",
    "env/dof/settings/use_lens_cats_eye",
    "env/dof/settings/use_distortion",
    "env/dof/settings/use_sharpen",
    "env/dof/settings/use_vignette",
    "env/dof/settings/debug_vignette",
    "env/dof/settings/use_noise",
    "env/dof/settings/noise_luminance_amount",
    "env/dof/settings/noise_grain_amount",
    "env/dof/settings/noise_lum_curve",
    "env/dof/settings/noise_speed",
];

/// The console/debug path reads integers for switches and the kernel; the
/// environment path reads floats and enables switches only when positive.
#[derive(Clone, Debug)]
pub struct ConsoleSettings {
    /// Same parameter indices as PARAMETER_NAMES; only fields actually read by
    /// the debug override are consumed. Noise values retain the existing state.
    pub values: [f32; 33],
    /// This CVar is an integer: converting it through f32 loses large values.
    pub kernel: i32,
    pub physical_focus: bool,
    /// Circle of confusion, focal length in mm, F-stop; used by debug override.
    pub physical_lens: [f32; 3],
}
pub struct SettingsInputs<'a> {
    pub environment: &'a [f32; 33],
    pub console: Option<&'a ConsoleSettings>,
    /// Global focus radius, sharp time, minimum and maximum sharpness.
    pub autofocus: [f32; 4],
    pub near_clip: f32,
    pub far_clip: f32,
    pub suppress_optics: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraSettings {
    words: [u32; 41],
}
#[derive(Clone, Debug)]
pub struct CameraEffectsFrame {
    pub settings: CameraSettings,
    pub near_clip: f32,
    pub far_clip: f32,
    pub opaque_enabled: bool,
    pub console_override: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombineParameters {
    pub camera: [f32; 4],
    pub near: [f32; 2],
    pub far: [f32; 4],
    pub distortion: [f32; 3],
    pub vignette: [f32; 4],
    pub noise: [f32; 4],
    pub centre: [f32; 2],
    /// Distortion, sharpening, vignette, noise, autofocus, near-DOF.
    pub options: [bool; 6],
}

impl CameraSettings {
    /// 3133f0 resets these fields only. Noise values and padding retain their
    /// bits until the environment updater writes them.
    pub fn reset(&mut self) {
        for (index, word) in [
            (0, 0xbf800000),
            (1, 0xbf800000),
            (2, 0xbf800000),
            (3, 0xbf800000),
            (6, 0x461c4600),
            (7, 0x42c80000),
            (8, 0x461c4000),
            (9, 0x461c4000),
            (10, 0x43fa0000),
            (11, 0x3f800000),
            (12, 0),
            (13, 0x3ccccccd),
            (14, 0x3eb33333),
            (15, 0x40800000),
            (16, 0x3c23d70a),
            (17, 0),
            (18, 0x3f800000),
            (19, 0x3f800000),
            (20, 0),
            (21, 0x3f800000),
            (22, 0x3f800000),
            (23, 0),
            (24, 0),
            (29, 0),
            (35, 0),
            (36, 0x3f800000),
            (37, 0),
            (38, 0x10000),
            (39, 0x3f000000),
            (40, 0x3f000000),
        ] {
            self.words[index] = word;
        }
        self.set_flag(0x14, true);
        self.set_flag(0x15, false);
        self.set_flag(0x16, true);
    }
    /// Seed owned by the camera/view producer, preserving all untouched lanes.
    pub fn from_words(words: [u32; 41]) -> Self {
        Self { words }
    }
    pub fn words(&self) -> &[u32; 41] {
        &self.words
    }
    fn get(&self, index: usize) -> f32 {
        f32::from_bits(self.words[index])
    }
    fn set(&mut self, index: usize, value: f32) {
        self.words[index] = value.to_bits();
    }
    fn flag(&self, offset: usize) -> bool {
        self.words[offset / 4] >> ((offset % 4) * 8) & 255 != 0
    }
    fn set_flag(&mut self, offset: usize, value: bool) {
        let shift = (offset % 4) * 8;
        self.words[offset / 4] =
            (self.words[offset / 4] & !(255 << shift)) | (u32::from(value) << shift);
    }
    pub fn bokeh_kernel(&self) -> i32 {
        self.words[35] as i32
    }
    pub fn bokeh_size(&self) -> f32 {
        self.get(36)
    }
    pub fn range_flags(&self) -> u32 {
        self.words[4]
    }
    pub fn special_alpha(&self) -> bool {
        self.flag(0x9b)
    }
    pub fn autofocus_enabled(&self) -> bool {
        self.flag(0x16)
    }

    /// Original mutable update: console override does not replace the complete
    /// record, and environment updates retain the camera's physical-focus mode.
    pub fn update(&mut self, input: SettingsInputs<'_>) {
        let debug = input.console.is_some();
        let v = input
            .console
            .map(|c| &c.values)
            .unwrap_or(input.environment);
        let switch = |index: usize| {
            if debug {
                cvt_i32(v[index]) != 0
            } else {
                v[index] > 0.0
            }
        };
        self.set_flag(0x14, switch(5));
        for (out, source) in [
            (7, 6),
            (8, 7),
            (9, 8),
            (12, 11),
            (16, 18),
            (17, 19),
            (18, 20),
            (20, 12),
            (21, 13),
            (22, 14),
            (23, 15),
            (24, 16),
            (29, 17),
        ] {
            self.set(out, v[source]);
        }
        if debug {
            self.set(10, v[9]);
            self.set(11, v[10]);
            let console = input.console.unwrap();
            self.set_flag(0x15, console.physical_focus);
            for i in 0..3 {
                self.set(13 + i, console.physical_lens[i]);
            }
            // Debug uses the physical F-stop for vignetting as well.
            self.set(19, console.physical_lens[2]);
        } else {
            for i in 0..4 {
                self.set(25 + i, v[29 + i]);
            }
            self.set(19, v[21]);
            if self.flag(0x14) {
                self.set(11, v[9]);
                self.set(10, v[10]);
            } else {
                self.set(11, self.get(9));
                self.set(10, self.get(8));
            }
            self.set(36, v[1]);
        }
        self.set_flag(0x16, switch(2));
        self.set(30, input.autofocus[0]);
        self.set(31, v[3]);
        self.set(
            32,
            if debug {
                input.autofocus[1]
            } else {
                v[3] * 1.5
            },
        );
        self.set(33, input.autofocus[2]);
        self.set(34, input.autofocus[3]);
        self.words[35] = input.console.map_or_else(|| cvt_i32(v[0]), |c| c.kernel) as u32;
        for (offset, index) in [
            (0x94, 23),
            (0x95, 22),
            (0x96, 24),
            (0x97, 25),
            (0x98, 26),
            (0x99, 28),
            (0x9a, 2),
        ] {
            self.set_flag(offset, switch(index));
        }
        if !self.flag(0x99) {
            for i in 25..29 {
                self.set(i, 1.0);
            }
        }
        if !self.flag(0x16) {
            self.set(31, 1.0);
            self.set(32, 1.0);
            self.set(33, 0.0);
            self.set(34, -1.0);
        } else {
            self.set(7, 0.0);
        }
        self.update_ranges(input.near_clip, input.far_clip);
        if input.suppress_optics {
            self.set_flag(0x96, false);
            self.set_flag(0x98, false);
        }
    }

    fn update_ranges(&mut self, near: f32, far: f32) {
        const MARGIN: f32 = 0.05;
        if !self.flag(0x15) {
            let (far_fuzzy_range, far_sharp_range) = if self.flag(0x14) {
                (self.get(10), self.get(11))
            } else {
                (self.get(8), self.get(9))
            };
            let focus = self.get(7);
            let near_range = self.get(8);
            let near_sharp_range = self.get(9);
            self.set(
                0,
                if focus <= 0.0 {
                    -(near_range + near_sharp_range + MARGIN)
                } else {
                    nonnegative(focus - (near_range + near_sharp_range + MARGIN))
                },
            );
            self.set(
                1,
                if focus > 0.0 {
                    nonnegative(focus - near_sharp_range)
                } else {
                    near_sharp_range
                },
            );
            let sharp = focus + far_sharp_range;
            let fuzzy = focus + far_fuzzy_range + far_sharp_range;
            self.set(2, sharp);
            self.set(3, fuzzy);
            let common = f32::from_bits(self.words[12] & 0x7fffffff) + fuzzy + MARGIN;
            self.set(6, common);
            if far > 0.0 {
                self.set(1, if self.get(1) <= far { self.get(1) } else { far });
                self.set(2, if sharp <= far { sharp } else { far });
                let capped = if self.get(2) <= fuzzy {
                    if far - 0.1 < fuzzy {
                        far - 0.1
                    } else {
                        fuzzy
                    }
                } else {
                    self.get(2)
                };
                self.set(3, capped);
                let focus = if focus <= self.get(2) {
                    focus
                } else {
                    self.get(2)
                };
                self.set(7, focus);
                self.set(
                    6,
                    if common <= far - MARGIN {
                        common
                    } else {
                        far - MARGIN
                    },
                );
                if far <= near_range {
                    self.set(1, far);
                }
            }
        } else {
            let focal = self.get(14);
            let focus = self.get(7) * 1000.0;
            if focal <= focus {
                let hyper = (focal * focal) / (self.get(15) * self.get(13));
                let near_sharp = (hyper * focus) / ((focus - focal) + hyper);
                self.set(1, near_sharp);
                if hyper <= focus {
                    let far_sharp = self.get(6) * 1000.0;
                    self.set(2, far_sharp);
                    self.set(3, far_sharp + 1000.0);
                } else {
                    let far_sharp = (hyper * focus) / (hyper - (focus - focal));
                    self.set(2, far_sharp);
                    self.set(
                        0,
                        ((focus - near_sharp) / (far_sharp - near_sharp)) * near_sharp,
                    );
                    self.set(
                        3,
                        ((far_sharp - focus) / (far_sharp - near_sharp)) * far_sharp + far_sharp,
                    );
                }
                self.set(1, near_sharp * 0.001);
                self.set(0, self.get(0) * 0.001);
                self.set(2, self.get(2) * 0.001);
                self.set(3, self.get(3) * 0.001);
            } else {
                self.set(2, 0.0);
                self.set(0, 0.0);
                let stop_coc = self.get(15) * self.get(13);
                self.set(
                    1,
                    focus
                        - (stop_coc * focus * (focus - focal))
                            / (focal * focal + (focus - focal) * stop_coc),
                );
                self.set(
                    3,
                    (stop_coc * focus * (focus - focal))
                        / (focal * focal - (focus - focal) * stop_coc)
                        + focus,
                );
            }
        }
        self.words[4] = 0;
        if self.get(1) != near && self.get(1) < self.get(0) {
            self.words[4] = 1;
            self.set(1, self.get(0) + MARGIN);
        }
        if self.get(2) != far && self.get(3) < self.get(2) {
            self.words[4] |= 2;
            self.set(3, self.get(2) + MARGIN);
        }
        if self.get(7) < 0.0 || (far > 0.0 && far <= self.get(7)) {
            self.set(1, far);
            self.set(2, far);
            self.words[4] |= 4;
        }
    }

    pub fn combine(&self, far_clip: f32) -> CombineParameters {
        let normalized = |i| {
            if self.flag(0x16) {
                self.get(i)
            } else {
                self.get(i) / self.get(6)
            }
        };
        CombineParameters {
            camera: [
                self.get(13),
                self.get(7) * 1000.0,
                self.get(14),
                self.get(15),
            ],
            near: [normalized(0), normalized(1)],
            far: [normalized(3), normalized(2), self.get(12), self.get(6)],
            distortion: [self.get(23), self.get(24), self.get(29)],
            vignette: [self.get(16), self.get(17), self.get(18), self.get(19)],
            noise: [self.get(25), self.get(26), self.get(27), self.get(28)],
            centre: [self.get(39), self.get(40)],
            options: [
                self.flag(0x96),
                self.flag(0x97),
                self.flag(0x98),
                self.flag(0x99),
                self.flag(0x9a),
                self.get(1) < far_clip,
            ],
        }
    }
    pub fn combine_material(&self, far_clip: f32) -> String {
        let flags = self.combine(far_clip).options;
        format!(
            "PostFX/DOF/dofCombine_{}_{}_{}_{}_{}_{}.material",
            u8::from(flags[5]),
            u8::from(flags[4]),
            u8::from(flags[3]),
            u8::from(flags[2]),
            u8::from(flags[1]),
            u8::from(flags[0])
        )
    }
}
fn nonnegative(value: f32) -> f32 {
    if 0.0 <= value {
        value
    } else {
        0.0
    }
}
fn cvt_i32(value: f32) -> i32 {
    if value.is_nan() || value >= 2147483648.0 || value < -2147483648.0 {
        i32::MIN
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reset_matches_executable_and_retains_untouched_fields() {
        let words: Vec<u32> = include_str!("../../tests/fixtures/camera_settings_reset.txt")
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        assert_eq!(words.len(), 82);
        let mut state = CameraSettings::from_words(words[..41].try_into().unwrap());
        state.reset();
        assert_eq!(state.words(), &words[41..]);
    }
    #[test]
    fn combine_materials_match_executable() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/camera_combine_materials.txt").lines() {
            let (mask, expected) = row.split_once(' ').unwrap();
            let mask: u32 = mask.parse().unwrap();
            let mut state = CameraSettings::from_words([0; 41]);
            state.set(1, if mask & 32 != 0 { 10.0 } else { 7000.0 });
            for i in 0..5 {
                state.set_flag(0x96 + i, mask & (1 << i) != 0);
            }
            assert_eq!(state.combine_material(7000.0), expected);
            count += 1;
        }
        assert_eq!(count, 64);
    }
    #[test]
    fn settings_match_executable() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/camera_settings.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let w: Vec<u32> = row
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            assert_eq!(w.len(), 123);
            let f = |i| f32::from_bits(w[i]);
            let env: [f32; 33] = std::array::from_fn(|i| f(41 + i));
            let debug = w[76] != 0;
            let mut state = CameraSettings::from_words(w[..41].try_into().unwrap());
            let console = ConsoleSettings {
                values: env,
                kernel: w[35] as i32,
                physical_focus: (w[5] >> 8) & 255 != 0,
                physical_lens: [f(13), f(14), f(15)],
            };
            state.update(SettingsInputs {
                environment: &env,
                console: debug.then_some(&console),
                autofocus: std::array::from_fn(|i| f(78 + i)),
                near_clip: f(74),
                far_clip: f(75),
                suppress_optics: w[77] != 0,
            });
            for i in 0..41 {
                assert_eq!(
                    state.words[i],
                    w[82 + i],
                    "case {count} word {i}: {} != {}",
                    state.get(i),
                    f(82 + i)
                );
            }
            count += 1;
        }
        assert_eq!(count, 512);
    }
}
