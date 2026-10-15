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
    pub parallel_slice_size: u32,
    pub depth_bias_clamp: f32,
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
fn parallel_slice_size(value: &str) -> u32 {
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
        .fold(0u32, |value, digit| {
            value
                .saturating_mul(10)
                .saturating_add(u32::from(digit - b'0'))
        });
    // strtol's signed overflow saturation gives the same result after this
    // positive range clamp. No digits returns zero; every negative clamps low.
    if negative {
        256
    } else {
        magnitude.clamp(256, 16384)
    }
}

pub fn shadow_settings(source: &str) -> Result<ShadowSettings, String> {
    Ok(ShadowSettings {
        quality_preset: shadow_quality_preset(source)?,
        parallel_slice_size: quoted_setting(source, "r_shadowParallelSliceSize")?
            .map(parallel_slice_size)
            .unwrap_or(crate::render_shadow::ORIGINAL_PARALLEL_SLICE_SIZE),
        // FUN_141372dc0 registers r_depthBiasClamp with this default.
        depth_bias_clamp: quoted_setting(source, "r_depthBiasClamp")?
            .map(|value| {
                value
                    .parse::<f32>()
                    .map_err(|_| "r_depthBiasClamp is not a float".to_owned())
            })
            .transpose()?
            .unwrap_or(1000.0),
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn shadow_startup_controls_keep_quality_missing_and_resolve_size() {
        assert_eq!(
            shadow_settings("").unwrap(),
            ShadowSettings {
                quality_preset: None,
                parallel_slice_size: 2048,
                depth_bias_clamp: 1000.0,
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
                parallel_slice_size: 1024,
                depth_bias_clamp: 250.5,
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
    }

    #[test]
    #[ignore = "requires installed original Saved Games configuration"]
    fn original_configuration_resolves_without_modification() {
        let (path, preset) = read_shadow_quality_preset().unwrap().unwrap();
        assert!(path.is_file());
        assert!(preset <= 6);
    }
}
