//! Read existing original-game graphics configuration; never create or modify it.
use std::path::PathBuf;

/// Read the existing original's canonical quoted-token configuration format.
/// Duplicate override order is deliberately not guessed.
fn quoted_setting<'a>(source: &'a str, name: &str) -> Result<Option<&'a str>, String> {
    let mut selected = None;
    for line in source.lines() {
        let mut tokens = line.split_whitespace();
        if tokens.next() != Some(name) {
            continue;
        }
        let value = tokens
            .next()
            .ok_or_else(|| format!("{name} has no value"))?;
        let value = value
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(|| format!("{name} is not a quoted numeric token"))?;
        if selected.replace(value).is_some() {
            return Err(format!(
                "duplicate {name}: override order is not yet traced"
            ));
        }
    }
    Ok(selected)
}

/// CVar values such as r_shadowCascadeFars contain several whitespace-separated
/// numbers inside one quoted token.
fn quoted_multi_value<'a>(source: &'a str, name: &str) -> Result<Option<&'a str>, String> {
    let mut selected = None;
    for line in source.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix(name) else {
            continue;
        };
        if rest.chars().next().is_some_and(|ch| !ch.is_whitespace()) {
            continue;
        }
        let rest = rest.trim_start();
        let value = rest
            .strip_prefix('"')
            .ok_or_else(|| format!("{name} is not a quoted value"))?;
        let end = value
            .find('"')
            .ok_or_else(|| format!("{name} has no closing quote"))?;
        if selected.replace(&value[..end]).is_some() {
            return Err(format!(
                "duplicate {name}: override order is not yet traced"
            ));
        }
    }
    Ok(selected)
}

/// The original `%f` conversions accept a valid numeric prefix and ignore
/// trailing characters, unlike Rust's all-or-nothing float parser.
fn original_float_prefix(value: &str) -> Option<f32> {
    let value = value.trim_start();
    (1..=value.len())
        .rev()
        .filter_map(|end| value.get(..end))
        .find_map(|prefix| prefix.parse::<f32>().ok())
}

fn original_four_float_cvar(value: &str) -> Option<[f32; 4]> {
    let mut values = value.split_ascii_whitespace();
    Some([
        original_float_prefix(values.next()?)?,
        original_float_prefix(values.next()?)?,
        original_float_prefix(values.next()?)?,
        original_float_prefix(values.next()?)?,
    ])
}

/// Missing quality stays missing: no guessed preset or capture fallback.
pub fn shadow_quality_preset(source: &str) -> Result<Option<u32>, String> {
    quoted_setting(source, "r_shadowQualityPreset")?
        .map(|value| {
            value
                .parse()
                .map_err(|_| "r_shadowQualityPreset is not an unsigned integer".into())
        })
        .transpose()
}

/// Actual startup controls for the native directional-shadow target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSettings {
    pub quality_preset: Option<u32>,
    /// Startup override for the native `r_shadowCascadeFars` CVar. None uses
    /// the executable's registered default `[4, 12, 30, 75]`.
    pub cascade_fars: Option<[f32; 4]>,
    pub parallel_slice_size: u32,
    pub depth_bias_clamp: f32,
    pub sort_shadows_ccw: bool,
    pub cascade_controls: ShadowCascadeControls,
}

/// Actual startup controls consumed by FUN_140330630 and its view owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowCascadeControls {
    pub tighten_parallel_shadows: bool,
    pub partial_dynamic_update: bool,
    pub partial_slice_update: bool,
}

impl Default for ShadowCascadeControls {
    fn default() -> Self {
        // FUN_1413731c0 / FUN_141372f70 / FUN_141372fc0
        // (chunk_000967.c:240,253,334) register DAT_141abefb8,
        // verified installed PE string "1".
        Self {
            tighten_parallel_shadows: true,
            partial_dynamic_update: true,
            partial_slice_update: true,
        }
    }
}

/// FUN_141377a90 registers 2048 and float bounds 256..16384. The integer
/// branch of FUN_140102cb0 uses FUN_1411ada38 (strtol, base 10), then clamps.
/// Only that branch's resulting slice size is represented here: not the
/// generic CVar registry, normalized string storage or subscriber callbacks.
/// Evidence: artifacts/ghidra-tool-output/950b4f3171a18db4e3f44d10.json,
/// artifacts/ghidra-tool-output/6a7f96e7d3743b6396abe760.json.
/// Remaining original scope: FUN_140100840 -> FUN_140101640 constructs the
/// registry, links DAT_1431c04b0, and owns copied/borrowed strings. Its registry
/// lifecycle (FUN_14013fb30/FUN_14013ff40) and FUN_140102cb0's other types and
/// subscriber list at +0x70 are not implemented by this read-only settings
/// loader. Trace those owners before implementing live CVar changes.
/// FUN_140102cb0 converts a float CVar with `_atof_l` (1411b578c), then
/// normalizes malformed trailing text through `FUN_140143c70`. Preserve the
/// numeric prefix rather than Rust's all-or-nothing `f32::parse` behavior.
fn original_float_cvar(value: &str) -> f32 {
    let value = value.trim_start();
    (0..=value.len())
        .rev()
        .filter_map(|end| value.get(..end))
        .find_map(|prefix| prefix.parse::<f64>().ok())
        .unwrap_or(0.0) as f32
}

/// Native Windows strtol(base10) result, before the caller's range clamp.
fn original_integer_cvar(value: &str) -> i32 {
    let mut bytes = value
        .bytes()
        .skip_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
        .peekable();
    let negative = bytes.peek() == Some(&b'-');
    if matches!(bytes.peek(), Some(b'+' | b'-')) {
        bytes.next();
    }
    let magnitude = bytes
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |value, digit| {
            value
                .saturating_mul(10)
                .saturating_add(i64::from(digit - b'0'))
        });
    let signed = if negative { -magnitude } else { magnitude };
    signed.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn parallel_slice_size(value: &str) -> u32 {
    original_integer_cvar(value).clamp(256, 16384) as u32
}

pub fn shadow_settings(source: &str) -> Result<ShadowSettings, String> {
    Ok(ShadowSettings {
        quality_preset: shadow_quality_preset(source)?,
        cascade_fars: quoted_multi_value(source, "r_shadowCascadeFars")?
            .and_then(original_four_float_cvar),
        parallel_slice_size: quoted_setting(source, "r_shadowParallelSliceSize")?
            .map(parallel_slice_size)
            .unwrap_or(crate::render_shadow::ORIGINAL_PARALLEL_SLICE_SIZE),
        // FUN_141372dc0 registers r_depthBiasClamp with this default.
        depth_bias_clamp: quoted_setting(source, "r_depthBiasClamp")?
            .map(original_float_cvar)
            .unwrap_or(1000.0),
        // FUN_1413647f0: native integer CVar default "1". The shadow
        // submission tests the resulting integer for nonzero, not ==1.
        sort_shadows_ccw: quoted_setting(source, "r_sortShadowsCCW")?
            .map(original_integer_cvar)
            .unwrap_or(1)
            != 0,
        cascade_controls: ShadowCascadeControls {
            tighten_parallel_shadows: quoted_setting(source, "r_tightenParallelShadows")?
                .map(original_integer_cvar)
                .unwrap_or(1)
                != 0,
            partial_dynamic_update: quoted_setting(source, "r_shadowParallelPartialDynaUpdate")?
                .map(original_integer_cvar)
                .unwrap_or(1)
                != 0,
            partial_slice_update: quoted_setting(source, "r_shadowParallelPartialSliceUpdate")?
                .map(original_integer_cvar)
                .unwrap_or(1)
                != 0,
        },
    })
}

/// Native local atlas/cache registrations: 141372e10..141372eb0 and
/// 141377730..141377960. These are startup values; preset low-resolution
/// division remains a separate quality-owner input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalShadowSettings {
    pub atlas: [u32; 3],
    pub cache: [u32; 3],
    pub minimum_resolution: i32,
    pub normal_resolution: i32,
    pub extra_resolution: i32,
}

pub fn local_shadow_settings(source: &str) -> Result<LocalShadowSettings, String> {
    let integer = |name: &str, default| {
        Ok::<_, String>(
            quoted_setting(source, name)?
                .map(original_integer_cvar)
                .unwrap_or(default),
        )
    };
    let dimensions = |prefix: &str, default_count, max_count| {
        Ok::<_, String>([
            integer(&format!("{prefix}SliceWidth"), 2048)?.clamp(256, 16384) as u32,
            integer(&format!("{prefix}SliceHeight"), 2048)?.clamp(256, 16384) as u32,
            integer(&format!("{prefix}SliceCnt"), default_count)?.clamp(1, max_count) as u32,
        ])
    };
    Ok(LocalShadowSettings {
        atlas: dimensions("r_shadowAtlas", 8, 32)?,
        cache: dimensions("r_shadowCache", 12, 48)?,
        minimum_resolution: integer("r_minShadowMapResolution", 128)?,
        normal_resolution: integer("r_maxShadowMapResolution", 1024)?,
        extra_resolution: integer("r_maxShadowMapExtraResolution", 2048)?,
    })
}

pub fn read_local_shadow_settings() -> Result<(PathBuf, LocalShadowSettings), String> {
    let (path, source) = read_configuration()?;
    Ok((
        path,
        local_shadow_settings(source.as_deref().unwrap_or(""))?,
    ))
}

/// Native image-manager filter codes (FUN_1402eb830's POINT/LINEAR/ANISO
/// table, also the token outputs of FUN_1402d9990).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ImageFilter {
    Linear = 0,
    Point = 1,
    Anisotropic = 2,
}

impl ImageFilter {
    pub fn from_native_token(value: &str) -> Result<Self, String> {
        if value.eq_ignore_ascii_case("linear") {
            Ok(Self::Linear)
        } else if value.eq_ignore_ascii_case("point") {
            Ok(Self::Point)
        } else if value.eq_ignore_ascii_case("aniso") {
            Ok(Self::Anisotropic)
        } else {
            Err(format!("bad r_textureFilter: '{value}'"))
        }
    }
}

/// Image-manager values consumed by native sampler creation. These are real
/// configured values, not the four-times anisotropy seen in one capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageSamplingSettings {
    pub min_filter: ImageFilter,
    pub mag_filter: ImageFilter,
    pub mip_filter: ImageFilter,
    pub max_anisotropy: f32,
    pub mip_lod_bias: f32,
}

/// Registrations 141369850/1413698a0/1413698f0/141369950/1413699b0:
/// anisotropy "1", bias "0", all three filters "LINEAR". FUN_1402eb830
/// clamps anisotropy below 1 to 1 (ordered compare, preserving NaN).
/// This parses startup input, NOT CVar dirty-bit notifications or image-cache
/// rebuilding. Those registry/renderer owners still must implement live edits.
/// Remaining original sampler path: FUN_1402d9300 parses sampler declarations
/// then dispatches PTR_PTR_1422fda18 vtable +0x30, now resolved offline to
/// FUN_140378280. Its descriptor arithmetic is implemented/tested separately
/// by render_state::native_sampler_descriptor (768 original executions).
/// Cache/mutex/allocation/device interfaces and FUN_1402d9850's address-token
/// decoder are not integrated. FUN_1402eb6f0's image-cache lookup and
/// FUN_1402eb750's allocation/locking owner are not covered by this parser.
pub fn image_sampling_settings(source: &str) -> Result<ImageSamplingSettings, String> {
    let filter = |name| {
        quoted_setting(source, name)?
            .map(ImageFilter::from_native_token)
            .transpose()
            .map(|value| value.unwrap_or(ImageFilter::Linear))
    };
    let anisotropy = quoted_setting(source, "image_anisotropy")?
        .map(original_float_cvar)
        .unwrap_or(1.0);
    Ok(ImageSamplingSettings {
        min_filter: filter("image_minFilter")?,
        mag_filter: filter("image_magFilter")?,
        mip_filter: filter("image_mipFilter")?,
        max_anisotropy: if anisotropy < 1.0 { 1.0 } else { anisotropy },
        mip_lod_bias: quoted_setting(source, "image_lodbias")?
            .map(original_float_cvar)
            .unwrap_or(0.0),
    })
}

/// Unlike quality presets, these defaults are established by the executable's
/// actual CVar registrations (141355b80/141355c20/141355bd0).
pub fn game_clock_controls(source: &str) -> Result<crate::game_clock::ClockControls, String> {
    let mut controls = crate::game_clock::ClockControls::default();
    if let Some(value) = quoted_setting(source, "g_gameFixedDt")? {
        controls.force_fixed_step = value
            .parse()
            .map_err(|_| "g_gameFixedDt is not an integer")?;
    }
    if let Some(value) = quoted_setting(source, "g_physicsStepTolerance")? {
        controls.physics_step_tolerance = value
            .parse()
            .map_err(|_| "g_physicsStepTolerance is not a float")?;
    }
    if let Some(value) = quoted_setting(source, "g_limitPhysicStepVariation")? {
        controls.limit_step_variation = value
            .parse()
            .map_err(|_| "g_limitPhysicStepVariation is not an integer")?;
    }
    Ok(controls)
}

/// Windows resolves redirected/localized Saved Games folders itself.
/// The subpath is the installed original's existing configuration location.
#[cfg(target_os = "windows")]
pub fn configuration_path() -> Result<PathBuf, String> {
    use std::{
        ffi::{c_void, OsString},
        os::windows::ffi::OsStringExt,
    };
    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            id: *const Guid,
            flags: u32,
            token: *mut c_void,
            path: *mut *mut u16,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(memory: *mut c_void);
    }
    // FOLDERID_SavedGames from the installed Windows SDK/windows-sys Shell constants.
    let saved_games = Guid {
        a: 0x4c5c32ff,
        b: 0xbb9d,
        c: 0x43b0,
        d: [0xb5, 0xb4, 0x2d, 0x72, 0xe5, 0x4e, 0xaa, 0xa4],
    };
    let mut pointer = std::ptr::null_mut();
    let result =
        unsafe { SHGetKnownFolderPath(&saved_games, 0, std::ptr::null_mut(), &mut pointer) };
    if result < 0 || pointer.is_null() {
        if !pointer.is_null() {
            unsafe { CoTaskMemFree(pointer.cast()) };
        }
        return Err(format!(
            "Saved Games folder lookup failed: HRESULT {result:#010x}"
        ));
    }
    let mut length = 0;
    unsafe {
        while *pointer.add(length) != 0 {
            length += 1;
        }
    }
    let root = unsafe { OsString::from_wide(std::slice::from_raw_parts(pointer, length)) };
    unsafe { CoTaskMemFree(pointer.cast()) };
    Ok(PathBuf::from(root).join("Arkane Studios/Dishonored2/base/dishonored2Config.cfg"))
}

#[cfg(not(target_os = "windows"))]
pub fn configuration_path() -> Result<PathBuf, String> {
    Err("original Saved Games configuration discovery requires Windows".into())
}

fn read_configuration() -> Result<(PathBuf, Option<String>), String> {
    let path = configuration_path()?;
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => Some(source),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    Ok((path, source))
}

pub fn read_shadow_quality_preset() -> Result<Option<(PathBuf, u32)>, String> {
    let (path, source) = read_configuration()?;
    match source {
        Some(source) => Ok(shadow_quality_preset(&source)?.map(|preset| (path, preset))),
        None => Ok(None),
    }
}

pub fn read_shadow_settings() -> Result<(PathBuf, ShadowSettings), String> {
    let (path, source) = read_configuration()?;
    Ok((path, shadow_settings(source.as_deref().unwrap_or(""))?))
}

pub fn read_game_clock_controls() -> Result<(PathBuf, crate::game_clock::ClockControls), String> {
    let (path, source) = read_configuration()?;
    Ok((path, game_clock_controls(source.as_deref().unwrap_or(""))?))
}

/// Registered view/LOD controls used when FUN_140351780 constructs the
/// ordinary-query and gather-caster distance descriptor. These are not the
/// separate r_shadowCullDistance model-shadow cutoff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewSettings {
    pub horizontal_fov_degrees: f32,
    pub lod_zoom_multiplier: f32,
    pub umbra_distance_imprecision: f32,
    pub umbra_accurate_threshold: f32,
}

pub fn view_settings(source: &str) -> Result<ViewSettings, String> {
    Ok(ViewSettings {
        // FUN_141392b10: default 80, limits at 141c8e8a4/8a8 = 65/110.
        horizontal_fov_degrees: quoted_setting(source, "ark_fieldOfView")?
            .map(original_float_cvar)
            .unwrap_or(80.0)
            .clamp(65.0, 110.0),
        // FUN_1413759f0: default "1.0f", min/max 358637bd/49742400.
        lod_zoom_multiplier: quoted_setting(source, "r_lodZoomMultiplicator")?
            .map(original_float_cvar)
            .unwrap_or(1.0)
            .clamp(f32::from_bits(0x3586_37bd), f32::from_bits(0x4974_2400)),
        //141379340/1413792f0 register unbounded float CVars "0.f"/"70.f".
        umbra_distance_imprecision: quoted_setting(source, "umbra_distance")?
            .map(original_float_cvar)
            .unwrap_or(0.0),
        umbra_accurate_threshold: quoted_setting(source, "umbra_accurateOcclusionThreshold")?
            .map(original_float_cvar)
            .unwrap_or(70.0),
    })
}

pub fn read_view_settings() -> Result<(PathBuf, ViewSettings), String> {
    let (path, source) = read_configuration()?;
    Ok((path, view_settings(source.as_deref().unwrap_or(""))?))
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_shadow_settings_preserve_registered_defaults_and_integer_conversion() {
        let defaults = super::local_shadow_settings("").unwrap();
        assert_eq!(
            defaults,
            super::LocalShadowSettings {
                atlas: [2048, 2048, 8],
                cache: [2048, 2048, 12],
                minimum_resolution: 128,
                normal_resolution: 1024,
                extra_resolution: 2048
            }
        );
        let settings=super::local_shadow_settings("r_shadowAtlasSliceWidth \"1\"\nr_shadowAtlasSliceHeight \"999999\"\nr_shadowAtlasSliceCnt \"99\"\nr_shadowCacheSliceCnt \"-3\"\nr_minShadowMapResolution \"12suffix\"\nr_maxShadowMapResolution \"-1\"\nr_maxShadowMapExtraResolution \"0\"").unwrap();
        assert_eq!(settings.atlas, [256, 16384, 32]);
        assert_eq!(settings.cache, [2048, 2048, 1]);
        assert_eq!(
            (
                settings.minimum_resolution,
                settings.normal_resolution,
                settings.extra_resolution
            ),
            (12, -1, 0)
        );
        assert!(super::local_shadow_settings(
            "r_shadowAtlasSliceCnt \"8\"\nr_shadowAtlasSliceCnt \"4\""
        )
        .is_err());
    }
    use super::*;
    #[test]
    fn image_sampling_uses_registered_defaults_and_native_lower_bound() {
        let defaults = image_sampling_settings("").unwrap();
        assert_eq!(
            defaults,
            ImageSamplingSettings {
                min_filter: ImageFilter::Linear,
                mag_filter: ImageFilter::Linear,
                mip_filter: ImageFilter::Linear,
                max_anisotropy: 1.0,
                mip_lod_bias: 0.0,
            }
        );
        let configured = image_sampling_settings(
            "image_minFilter \"ANISO\"\nimage_magFilter \"Point\"\nimage_mipFilter \"linear\"\nimage_anisotropy \"4\"\nimage_lodbias \"-0.75\"\n"
        ).unwrap();
        assert_eq!(configured.min_filter, ImageFilter::Anisotropic);
        assert_eq!(configured.mag_filter, ImageFilter::Point);
        assert_eq!(configured.mip_filter, ImageFilter::Linear);
        assert_eq!(configured.max_anisotropy, 4.0);
        assert_eq!(configured.mip_lod_bias, -0.75);
        for value in ["-9", "0", "0.75", "garbage"] {
            let source = format!("image_anisotropy \"{value}\"\n");
            assert_eq!(
                image_sampling_settings(&source).unwrap().max_anisotropy,
                1.0
            );
        }
        assert!(image_sampling_settings("image_anisotropy \"NaN\"\n")
            .unwrap()
            .max_anisotropy
            .is_nan());
        assert!(image_sampling_settings("image_minFilter \"invalid\"\n").is_err());
        assert!(
            image_sampling_settings("image_anisotropy \"1\"\nimage_anisotropy \"4\"\n").is_err()
        );
    }

    #[test]
    fn quoted_config_does_not_invent_missing_or_invalid_quality() {
        assert_eq!(
            shadow_quality_preset("r_shadowQualityPreset \"5\"\n").unwrap(),
            Some(5)
        );
        assert_eq!(
            shadow_quality_preset(
                "r_shadowQualityPresetExtra \"5\"\n// r_shadowQualityPreset \"1\"\n"
            )
            .unwrap(),
            None
        );
        assert!(shadow_quality_preset(
            "r_shadowQualityPreset \"0\"\nr_shadowQualityPreset \"6\"\n"
        )
        .is_err());
        assert!(shadow_quality_preset("r_shadowQualityPreset 5").is_err());
        assert!(shadow_quality_preset("r_shadowQualityPreset \"-1\"").is_err());
        assert!(shadow_quality_preset("r_shadowQualityPreset").is_err());
        assert_eq!(shadow_quality_preset("").unwrap(), None);
    }
    #[test]
    fn clock_configuration_uses_registered_defaults_and_actual_overrides() {
        let defaults = crate::game_clock::ClockControls::default();
        assert_eq!(game_clock_controls("").unwrap(), defaults);
        assert_eq!(
            game_clock_controls("// g_gameFixedDt \"1\"\ng_gameFixedDtExtra \"1\"").unwrap(),
            defaults
        );
        let actual = game_clock_controls("g_gameFixedDt \"2\"\ng_physicsStepTolerance \"0.125\"\ng_limitPhysicStepVariation \"1\"").unwrap();
        assert_eq!(actual.force_fixed_step, 2);
        assert_eq!(actual.physics_step_tolerance, 0.125);
        assert_eq!(actual.limit_step_variation, 1);
        for bad in [
            "g_gameFixedDt",
            "g_gameFixedDt 1",
            "g_gameFixedDt \"bad\"",
            "g_physicsStepTolerance \"bad\"",
            "g_gameFixedDt \"0\"\ng_gameFixedDt \"1\"",
        ] {
            assert!(game_clock_controls(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn slice_size_uses_native_integer_prefix_and_range() {
        for (source, expected) in [
            ("2048", 2048),
            ("+1024", 1024),
            ("257", 257),
            ("16384", 16384),
            ("16385", 16384),
            ("-1", 256),
            ("0", 256),
            ("255", 256),
            ("1024.9", 1024),
            ("1024suffix", 1024),
            ("0x1000", 256),
            ("invalid", 256),
            ("", 256),
            ("2147483648", 16384),
            ("999999999999999999999999", 16384),
            ("-999999999999999999999999", 256),
            ("\u{b}\t 512", 512),
        ] {
            assert_eq!(parallel_slice_size(source), expected, "{source:?}");
        }
    }

    #[test]
    fn native_shadow_sort_control_retains_integer_prefix_and_nonzero_semantics() {
        for (value, expected) in [
            ("0", false),
            ("1", true),
            ("-2", true),
            ("+0junk", false),
            ("2junk", true),
            ("invalid", false),
            ("999999999999999999999999", true),
            ("-999999999999999999999999", true),
        ] {
            let source = format!("r_sortShadowsCCW \"{value}\"");
            assert_eq!(
                shadow_settings(&source).unwrap().sort_shadows_ccw,
                expected,
                "{value}"
            );
        }
    }

    #[test]
    fn cascade_controls_preserve_native_names_defaults_and_integer_semantics() {
        assert_eq!(
            shadow_settings("").unwrap().cascade_controls,
            ShadowCascadeControls {
                tighten_parallel_shadows: true,
                partial_dynamic_update: true,
                partial_slice_update: true,
            }
        );
        let settings = shadow_settings(
            "r_tightenParallelShadows \"0\"\nr_shadowParallelPartialSliceUpdate \"-2junk\"\nr_shadowParallelPartialDynaUpdate \"0\""
        ).unwrap();
        assert_eq!(
            settings.cascade_controls,
            ShadowCascadeControls {
                tighten_parallel_shadows: false,
                partial_dynamic_update: false,
                partial_slice_update: true,
            }
        );
        assert!(
            shadow_settings("r_tightenParallelShadows \"1\"\nr_tightenParallelShadows \"0\"")
                .is_err()
        );
    }

    #[test]
    fn native_cascade_far_cvar_parses_four_sscanf_style_float_prefixes() {
        assert_eq!(shadow_settings("").unwrap().cascade_fars, None);
        assert_eq!(
            shadow_settings("r_shadowCascadeFars \"3 15 45 90\"")
                .unwrap()
                .cascade_fars,
            Some([3.0, 15.0, 45.0, 90.0])
        );
        assert_eq!(
            shadow_settings("r_shadowCascadeFars \"4 12 30 75tail\"")
                .unwrap()
                .cascade_fars,
            Some([4.0, 12.0, 30.0, 75.0])
        );
        assert_eq!(
            shadow_settings("r_shadowCascadeFars \"4 12 invalid 75\"")
                .unwrap()
                .cascade_fars,
            None
        );
        assert!(shadow_settings(
            "r_shadowCascadeFars \"4 12 30 75\"\nr_shadowCascadeFars \"1 2 3 4\""
        )
        .is_err());
    }

    #[test]
    fn shadow_startup_controls_keep_quality_missing_and_resolve_size() {
        assert_eq!(
            shadow_settings("").unwrap(),
            ShadowSettings {
                quality_preset: None,
                cascade_fars: None,
                parallel_slice_size: 2048,
                depth_bias_clamp: 1000.0,
                sort_shadows_ccw: true,
                cascade_controls: ShadowCascadeControls::default(),
            }
        );
        let actual = shadow_settings(
            "r_shadowQualityPreset \"5\"\nr_shadowParallelSliceSize \"1024\"\nr_depthBiasClamp \"250.5\"\n",
        )
        .unwrap();
        assert_eq!(
            actual,
            ShadowSettings {
                quality_preset: Some(5),
                cascade_fars: None,
                parallel_slice_size: 1024,
                depth_bias_clamp: 250.5,
                sort_shadows_ccw: true,
                cascade_controls: ShadowCascadeControls::default(),
            }
        );
        assert_eq!(
            shadow_settings("r_shadowParallelSliceSize \"-1\"")
                .unwrap()
                .parallel_slice_size,
            256
        );
        assert!(shadow_settings(
            "r_shadowParallelSliceSize \"512\"\nr_shadowParallelSliceSize \"1024\""
        )
        .is_err());
        assert!(shadow_settings("r_shadowParallelSliceSize 512").is_err());
        assert!(shadow_settings("r_depthBiasClamp 100").is_err());
        assert!(shadow_settings("r_depthBiasClamp \"100\"\nr_depthBiasClamp \"200\"").is_err());
        assert_eq!(
            shadow_settings("r_depthBiasClamp \"250.5f\"")
                .unwrap()
                .depth_bias_clamp,
            250.5
        );
        assert_eq!(
            shadow_settings("r_depthBiasClamp \"not-a-number\"")
                .unwrap()
                .depth_bias_clamp,
            0.0
        );
    }

    #[test]
    #[ignore = "requires installed original Saved Games configuration"]
    fn original_configuration_resolves_without_modification() {
        let (path, preset) = read_shadow_quality_preset().unwrap().unwrap();
        assert!(path.is_file());
        assert!(preset <= 6);
    }
}
