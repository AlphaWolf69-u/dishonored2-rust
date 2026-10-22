//! Presence-aware 88e220 tracks for prepared parameters. Integer interpolation
//! and lens-profile publication differ from ordinary floating interpolation.
use super::environment::{EnvironmentSource, MIN_BLEND_DURATION};

/// Track retirement compares the four stored words, including signed zeros.
#[derive(Clone, Copy)]
pub(crate) struct FloatParameter(pub bevy_math::Vec4);
impl PartialEq for FloatParameter {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_array().map(f32::to_bits) == other.0.to_array().map(f32::to_bits)
    }
}

pub(crate) struct ParameterTrack<T> {
    registered_default: T,
    current: T,
    transition: Option<Transition<T>>,
}
struct Transition<T> {
    source: Option<EnvironmentSource>,
    start: T,
    target: T,
    duration: f32,
    elapsed: f32,
}
impl<T: Clone + PartialEq> ParameterTrack<T> {
    pub(crate) fn new(registered_default: T) -> Self {
        Self {
            current: registered_default.clone(),
            registered_default,
            transition: None,
        }
    }
    pub(crate) fn update(
        &mut self,
        selection: Option<(EnvironmentSource, T)>,
        duration: f32,
        delta: f32,
        parameters_initialized: bool,
        mut blend: impl FnMut(&T, &T, f32) -> T,
    ) -> &T {
        let source = selection.as_ref().map(|v| v.0);
        let changed = self.transition.as_ref().is_some_and(|v| v.source != source);
        let new_world = self.transition.is_none() && source == Some(EnvironmentSource::World);
        if changed || (self.transition.is_none() && source.is_some()) {
            self.transition = Some(Transition {
                source,
                start: self.current.clone(),
                target: selection
                    .map(|v| v.1)
                    .unwrap_or_else(|| self.registered_default.clone()),
                duration: if parameters_initialized && !new_world {
                    duration
                } else {
                    0.0
                },
                elapsed: 0.0,
            });
        }
        let mut retire = false;
        if let Some(track) = &mut self.transition {
            if track.elapsed <= track.duration {
                track.elapsed += delta;
                let fraction =
                    (track.elapsed / track.duration.max(MIN_BLEND_DURATION)).clamp(0.0, 1.0);
                self.current = blend(&track.start, &track.target, fraction);
                retire = track.source.is_none()
                    && track.elapsed >= track.duration
                    && self.current == self.registered_default;
            }
        }
        if retire {
            self.transition = None;
        }
        &self.current
    }
}

/// Ordinary image parameters select the target at fraction >= 0.5. The
/// lens dirt and profile parameters have separate publication rules.
pub struct ImageTrack<T>(ParameterTrack<T>);
impl<T: Clone + PartialEq> ImageTrack<T> {
    pub fn new(default: T) -> Self {
        Self(ParameterTrack::new(default))
    }
    pub fn update(
        &mut self,
        selection: Option<(EnvironmentSource, T)>,
        duration: f32,
        delta: f32,
        initialized: bool,
    ) -> &T {
        self.0.update(
            selection,
            duration,
            delta,
            initialized,
            |start, target, t| {
                if t >= 0.5 {
                    target.clone()
                } else {
                    start.clone()
                }
            },
        )
    }
}

/// Fog curves retain ordinary image-track state while publishing the target
/// resource and fraction separately (88e220, manager slots 24d00/24d08).
/// A target's equality with the current image does not force weight one.
pub struct FogLookupTrack<T> {
    track: ParameterTrack<T>,
    target: T,
    weight: f32,
}
impl<T: Clone + PartialEq> FogLookupTrack<T> {
    /// Registered defaults and the owner's initial publication are distinct.
    pub fn new(registered_default: T, initial_target: T, initial_weight: f32) -> Self {
        Self {
            track: ParameterTrack::new(registered_default),
            target: initial_target,
            weight: initial_weight,
        }
    }
    pub fn update(
        &mut self,
        selection: Option<(EnvironmentSource, T)>,
        duration: f32,
        delta: f32,
        initialized: bool,
    ) {
        let (target, weight) = (&mut self.target, &mut self.weight);
        self.track.update(
            selection,
            duration,
            delta,
            initialized,
            |start, end, fraction| {
                *target = end.clone();
                *weight = fraction;
                if fraction >= 0.5 {
                    end.clone()
                } else {
                    start.clone()
                }
            },
        );
    }
    pub fn target(&self) -> &T {
        &self.target
    }
    pub fn weight(&self) -> f32 {
        self.weight
    }
}

/// Integer subtypes preserve the executable's subtraction/truncation order;
/// using ordinary float lerp here produces different intermediate integers.
pub fn blend_unsigned(start: u32, target: u32, fraction: f32) -> u32 {
    ((target as f32 - start as f32 * fraction) as i64 as u32).wrapping_add(start)
}
pub fn blend_signed(start: i32, target: i32, fraction: f32) -> i32 {
    let delta = target as f32 - start as f32 * fraction;
    (crate::entities::integer_word(delta.to_bits(), 0x30) as i32).wrapping_add(start)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LensProfileBlend<T> {
    pub from: T,
    pub to: T,
    pub weight: f32,
}
pub struct LensProfileTrack<T> {
    track: ParameterTrack<T>,
    target: T,
    weight: f32,
}
impl<T: Clone + PartialEq> LensProfileTrack<T> {
    /// The manager initializes its target with the intrinsic white image and
    /// weight zero, independently of the registered LensColor_From default.
    pub fn new(registered_default: T, white: T) -> Self {
        Self {
            track: ParameterTrack::new(registered_default),
            target: white,
            weight: 0.0,
        }
    }
    pub fn update(
        &mut self,
        selection: Option<(EnvironmentSource, T)>,
        duration: f32,
        delta: f32,
        parameters_initialized: bool,
    ) {
        let (published_target, weight) = (&mut self.target, &mut self.weight);
        self.track.update(
            selection,
            duration,
            delta,
            parameters_initialized,
            |start, target, fraction| {
                *published_target = target.clone();
                *weight = if start == target { 1.0 } else { fraction };
                if *weight == 1.0 {
                    target.clone()
                } else {
                    start.clone()
                }
            },
        );
    }
    pub fn value(&self) -> LensProfileBlend<T> {
        LensProfileBlend {
            from: self.track.current.clone(),
            to: self.target.clone(),
            weight: self.weight,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(
        fixture: &str,
        mut run: impl FnMut(bool, Option<(EnvironmentSource, u32)>, f32, f32, bool) -> [u32; 3],
    ) -> usize {
        let mut count = 0;
        for line in fixture.lines() {
            if line == "RESET" {
                run(true, None, 0.0, 0.0, false);
                continue;
            }
            let v: Vec<_> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let mut selection = if v[0] != 0 {
                Some((EnvironmentSource::World, 101))
            } else {
                None
            };
            let mut duration = 0.6;
            if v[1] > 0 {
                selection = Some((EnvironmentSource::Area(0), v[2]));
                duration = f32::from_bits(v[5]);
            }
            if v[1] > 1 {
                selection = Some((EnvironmentSource::Area(1), v[3]));
                duration = f32::from_bits(v[6]);
            }
            if v[9] != 0 {
                selection = Some((EnvironmentSource::Secondary(0), v[9]));
                duration = 0.0;
            }
            let actual = run(false, selection, duration, f32::from_bits(v[4]), v[8] != 0);
            assert_eq!(actual, &v[18..21], "update{count}: {line}");
            count += 1;
        }
        count
    }
    #[test]
    fn ordinary_images_match_executable_updates() {
        let mut track = ImageTrack::new(100);
        assert_eq!(
            rows(
                include_str!("../../tests/fixtures/environment_image_track.txt"),
                |reset, selection, duration, delta, initialized| {
                    if reset {
                        track = ImageTrack::new(100);
                        return [0; 3];
                    }
                    [*track.update(selection, duration, delta, initialized), 0, 0]
                }
            ),
            55
        );
    }
    #[test]
    fn lens_profiles_match_executable_updates() {
        let mut track = LensProfileTrack::new(100, 200);
        assert_eq!(
            rows(
                include_str!("../../tests/fixtures/lens_profile_track.txt"),
                |reset, selection, duration, delta, initialized| {
                    if reset {
                        track = LensProfileTrack::new(100, 200);
                        return [0; 3];
                    }
                    track.update(selection, duration, delta, initialized);
                    let v = track.value();
                    [v.from, v.to, v.weight.to_bits()]
                }
            ),
            51
        );
    }

    #[test]
    fn fog_lookup_targets_match_executable_updates() {
        let mut track = FogLookupTrack::new(100, 200, 0.);
        assert_eq!(
            rows(
                include_str!("../../tests/fixtures/fog_lookup_track.txt"),
                |reset, selection, duration, delta, initialized| {
                    if reset {
                        track = FogLookupTrack::new(100, 200, 0.);
                        return [0; 3];
                    }
                    track.update(selection, duration, delta, initialized);
                    [
                        track.track.current,
                        *track.target(),
                        track.weight().to_bits(),
                    ]
                }
            ),
            55
        );
    }
    #[test]
    fn integers_match_executable_updates() {
        for signed in [false, true] {
            let fixture = if signed {
                include_str!("../../tests/fixtures/environment_signed_track.txt")
            } else {
                include_str!("../../tests/fixtures/environment_unsigned_track.txt")
            };
            let mut track = ParameterTrack::new(100);
            assert_eq!(
                rows(fixture, |reset, selection, duration, delta, initialized| {
                    if reset {
                        track = ParameterTrack::new(100);
                        return [0; 3];
                    }
                    let value = track.update(
                        selection,
                        duration,
                        delta,
                        initialized,
                        |start, target, t| {
                            if signed {
                                blend_signed(*start as i32, *target as i32, t) as u32
                            } else {
                                blend_unsigned(*start, *target, t)
                            }
                        },
                    );
                    [*value, 0, 0]
                }),
                56
            );
        }
    }
    #[test]
    fn float_retirement_preserves_executable_word_comparison() {
        let default = FloatParameter(bevy_math::Vec4::new(-0.0, 0.0, 0.0, 0.0));
        let mut track = ParameterTrack::new(default);
        assert_eq!(
            rows(
                include_str!("../../tests/fixtures/environment_float_track.txt"),
                |reset, selection, duration, delta, initialized| {
                    if reset {
                        track = ParameterTrack::new(default);
                        return [0; 3];
                    }
                    let selection = selection.map(|(source, value)| {
                        (
                            source,
                            FloatParameter(bevy_math::Vec4::new(
                                if source == EnvironmentSource::World {
                                    5.0
                                } else {
                                    value as f32
                                },
                                0.0,
                                0.0,
                                0.0,
                            )),
                        )
                    });
                    let current = track.update(
                        selection,
                        duration,
                        delta,
                        initialized,
                        |start, target, t| FloatParameter((target.0 - start.0) * t + start.0),
                    );
                    [current.0.x.to_bits(), 0, 0]
                }
            ),
            58
        );
    }
}
