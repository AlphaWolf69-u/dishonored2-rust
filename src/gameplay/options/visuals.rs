//! `arkMenuSectionOptionVisuals` (vtable 0x141b71478), notes-options.md 6.
use super::decl::{self, widget, SectionDecl, Setting};
use super::{apply_cvar, load_cvar, movie_value, read_back, refresh, Effect, Env, Kind, MessageBox, Result, Section, CURRENT};
use crate::assets::entities::Value as DeclValue;
use crate::flash::avm2::value::Value;
use crate::platform::cvars::float_string;

/// `ARK_VISUALSETTING_*`.
const RESOLUTION: usize = 0;
const MONITOR: usize = 1;
const WINDOWED: usize = 2;
const GAMMA: usize = 3;
const VIDEOCARD: usize = 6;
const PRESET: usize = 7;
const FOG_BLUR: usize = 14;
const ANTIALIASING: usize = 15;
const AA_SHARPNESS: usize = 16;
const VSYNC: usize = 24;
const RES_SCALE: usize = 25;
const RES_SCALE_MANUAL: usize = 26;
const RES_SCALE_QUALITY: usize = 27;
const FRAMERATE: usize = 28;
const SWAPBUFFER: usize = 29;
const HBAO: usize = 30;
const SETTINGS: usize = 31;
/// `arkQualityPresets_t` AUTO.
const AUTO: i32 = 6;
const QUALITY_NAMES: [&str; 7] =
    ["VERY_LOW_QUALITY", "LOW_QUALITY", "MEDIUM_QUALITY", "HIGH_QUALITY", "VERY_HIGH_QUALITY", "ULTRA_QUALITY", "AUTO_QUALITY"];

/// What the renderer reports (renderer vfuncs +0x2f0, +0x188, +0x170,
/// +0x278, +0x1a0 and `FUN_140345f40`).
#[derive(Clone, Debug, Default)]
pub struct Display {
    /// The renderer's monitor table (`FUN_140345b40`).
    pub table: crate::platform::display_modes::Monitors,
    pub modes: Vec<(i32, i32)>,
    pub monitors: Vec<String>,
    pub current_monitor: i32,
    /// The mode `r_mode` names: (width, height, monitor).
    pub current_mode: Option<(i32, i32, i32)>,
    /// (id string, name) per adapter.
    pub adapters: Vec<(String, String)>,
    /// `FUN_140345f40`: the refresh rate, -1 when unknown.
    pub refresh_rate: f32,
}

impl Display {
    /// The renderer state for the current cvars. The adapter list
    /// (vfunc +0x1a0) is filtered by the D3D11 init's acceptance test, which
    /// the rewrite does not run; VIDEOCARD is main-menu only, so the list
    /// stays empty in the pause menu.
    pub fn query(cvars: &crate::platform::cvars::CVars) -> Display {
        use crate::platform::display_modes as dm;
        let table = dm::monitors();
        let r_mode = cvars.int("r_mode").unwrap_or(-1);
        // ShowGameWindow: the current monitor is `r_mode`'s (mode 0 when invalid).
        let current_monitor = table.mode(r_mode).or_else(|| table.mode(0)).map_or(0, |m| m.2);
        let handle = table.monitors.get(current_monitor as usize).map_or(0, |m| m.handle);
        let modes = table.menu_modes(current_monitor, &dm::dxgi_modes(handle));
        let refresh_rate = match cvars.int("r_fullscreen").unwrap_or(0) {
            0 => table.monitors.get(current_monitor as usize).map_or(-1.0, dm::registry_refresh),
            // Exclusive fullscreen reports the DXGI mode the swapchain resize
            // picked (`FUN_14036aa40`); the rewrite has no exclusive mode.
            1 => -1.0,
            2 => table.borderless_refresh(r_mode),
            _ => -1.0,
        };
        Display {
            monitors: dm::names(&table),
            current_mode: table.mode(r_mode),
            current_monitor,
            modes,
            adapters: Vec::new(),
            refresh_rate,
            table,
        }
    }
}

/// `m_shadowsSettings[q]` (0x2c), ctor defaults `FUN_140509be0`.
#[derive(Clone, Debug)]
struct Shadows {
    cull_distance: f32,
    parallel_partial_dyna_update: bool,
    spot_omni_part_update_dyna_threshold: f32,
    spot_omni_part_update_dyna_sqr_distance_threshold: f32,
    parallel_resolution_divider_level: i32,
    spot_omni_low_resolution_divider_enabled: bool,
    shadows_32_bits: bool,
    parallel_slice_cnt: i32,
    cascade_fars: [f32; 4],
}

impl Default for Shadows {
    fn default() -> Self {
        Shadows {
            cull_distance: 25.0,
            parallel_partial_dyna_update: true,
            spot_omni_part_update_dyna_threshold: 0.5,
            spot_omni_part_update_dyna_sqr_distance_threshold: 144.0,
            parallel_resolution_divider_level: 1,
            spot_omni_low_resolution_divider_enabled: true,
            shadows_32_bits: false,
            parallel_slice_cnt: 3,
            cascade_fars: [12.0, 30.0, 75.0, 7000.0],
        }
    }
}

/// `m_waterSettings[q]` (0x10), ctor defaults `FUN_140509fe0`.
#[derive(Clone, Debug)]
struct Water {
    tile_size: f32,
    geometry_details: f32,
    skip_ocean_ssr: bool,
    screen_space_max_steps: i32,
}

impl Default for Water {
    fn default() -> Self {
        Water { tile_size: 1.0, geometry_details: 512.0, skip_ocean_ssr: false, screen_space_max_steps: 64 }
    }
}

/// `m_textureSetting[q]` (0x14).
#[derive(Clone, Debug, Default)]
struct Texture {
    max_mip_size: i32,
}

/// The per-quality tables of the visuals decl.
#[derive(Clone, Debug)]
struct Tables {
    water: [Water; 7],
    lod: [[f32; 4]; 7],
    umbra: [f32; 7],
    shadows: [Shadows; 7],
    view_distance: [f32; 7],
    decals: [i32; 7],
    textures: [Texture; 7],
}

fn quality_rows<'a>(edit: &'a [(String, DeclValue)], name: &str) -> Vec<(usize, &'a DeclValue)> {
    decl::scope(edit, name)
        .map(|s| {
            s.iter()
                .filter_map(|(k, v)| {
                    let name = decl::enum_key(k)?;
                    Some((QUALITY_NAMES.iter().position(|q| *q == name)?, v))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn atom_f32(v: &DeclValue, default: f32) -> f32 {
    match v {
        DeclValue::Atom(a) => decl::unquote(a).parse().unwrap_or(default),
        _ => default,
    }
}

impl Tables {
    /// `FUN_14050a140`: the defaults, then the decl rows.
    fn read(edit: &[(String, DeclValue)]) -> Tables {
        let mut t = Tables {
            water: Default::default(),
            lod: [[3.0, 6.0, 12.0, 20.0]; 7],
            umbra: [1.0; 7],
            shadows: Default::default(),
            view_distance: [7000.0; 7],
            decals: [32; 7],
            textures: Default::default(),
        };
        for (q, v) in quality_rows(edit, "m_waterSettings") {
            if let DeclValue::Scope(f) = v {
                let w = &mut t.water[q];
                w.tile_size = decl::float(f, "m_tileSize", w.tile_size);
                w.geometry_details = decl::float(f, "m_geometryDetails", w.geometry_details);
                w.skip_ocean_ssr = decl::boolean(f, "m_skipOceanSSR", w.skip_ocean_ssr);
                w.screen_space_max_steps = decl::int(f, "m_screenSpaceMaxSteps", w.screen_space_max_steps);
            }
        }
        for (q, v) in quality_rows(edit, "m_dynamicLODSettings") {
            if let DeclValue::Scope(f) = v {
                for (i, name) in ["m_lod0", "m_lod1", "m_lod2", "m_lod3"].iter().enumerate() {
                    t.lod[q][i] = decl::float(f, name, t.lod[q][i]);
                }
            }
        }
        for (q, v) in quality_rows(edit, "m_umbraGateSettings") {
            t.umbra[q] = atom_f32(v, t.umbra[q]);
        }
        for (q, v) in quality_rows(edit, "m_viewDistanceSettings") {
            t.view_distance[q] = atom_f32(v, t.view_distance[q]);
        }
        for (q, v) in quality_rows(edit, "m_decalsSettings") {
            t.decals[q] = atom_f32(v, t.decals[q] as f32) as i32;
        }
        for (q, v) in quality_rows(edit, "m_shadowsSettings") {
            if let DeclValue::Scope(f) = v {
                let s = &mut t.shadows[q];
                s.cull_distance = decl::float(f, "m_shadowCullDistance", s.cull_distance);
                s.parallel_partial_dyna_update = decl::boolean(f, "m_shadowParallelPartialDynaUpdate", s.parallel_partial_dyna_update);
                s.spot_omni_part_update_dyna_threshold =
                    decl::float(f, "m_shadowSpotAndOmniPartUpdateDynaThreshold", s.spot_omni_part_update_dyna_threshold);
                s.spot_omni_part_update_dyna_sqr_distance_threshold = decl::float(
                    f,
                    "m_shadowSpotAndOmniPartUpdateDynaSqrDistanceThreshold",
                    s.spot_omni_part_update_dyna_sqr_distance_threshold,
                );
                s.parallel_resolution_divider_level =
                    decl::int(f, "m_shadowParallelResolutionDividerLevel", s.parallel_resolution_divider_level);
                s.spot_omni_low_resolution_divider_enabled =
                    decl::boolean(f, "m_shadowSpotAndOmniLowResolutionDividerEnabled", s.spot_omni_low_resolution_divider_enabled);
                s.shadows_32_bits = decl::boolean(f, "m_shadows32Bits", s.shadows_32_bits);
                s.parallel_slice_cnt = decl::int(f, "m_shadowParallelSliceCnt", s.parallel_slice_cnt);
                if let Some(c) = decl::scope(f, "m_shadowCascadeFars") {
                    for (i, axis) in ["x", "y", "z", "w"].iter().enumerate() {
                        s.cascade_fars[i] = decl::float(c, axis, s.cascade_fars[i]);
                    }
                }
            }
        }
        for (q, v) in quality_rows(edit, "m_textureSetting") {
            if let DeclValue::Scope(f) = v {
                t.textures[q].max_mip_size = decl::int(f, "m_textureMaxMipSize", t.textures[q].max_mip_size);
            }
        }
        t
    }
}

#[derive(Debug)]
pub struct State {
    tables: Tables,
    /// `+0x238`: the gamma value the link was filled with.
    gamma: i32,
    /// `+0x240..0x243`: r_skipColorGrading/Bloom/LensFlares/ToneMapping
    /// saved while the gamma screen shows.
    saved_skips: Option<[bool; 4]>,
    /// `+0x244`: widget values when the section was filled.
    snapshot: [i32; SETTINGS],
    /// `+0x230`: the video-card box was shown.
    video_card_box_shown: bool,
    /// `+0x2c0` / `+0x2c1`: texture preset 0 / texture cap lowered.
    texture_lowest: bool,
    texture_cap_lowered: bool,
    /// `+0x308`, `+0x309`, `+0x30a`, `+0x30b`.
    display_pending: bool,
    keep_changes_box: bool,
    display_changed: bool,
    two_step: bool,
    /// `+0x2c4..+0x2d4`: the mode before a try.
    saved_mode: Option<(i32, i32, i32, i32, bool)>,
    pub display: Display,
}

impl State {
    /// Ctor `FUN_14050a480`, ending with `FUN_14050df80`.
    pub fn new(decl: &SectionDecl, console: &mut crate::platform::console::Console) -> State {
        let mut st = State {
            tables: Tables::read(&decl.edit),
            gamma: 0,
            saved_skips: None,
            snapshot: [0; SETTINGS],
            video_card_box_shown: false,
            texture_lowest: false,
            texture_cap_lowered: false,
            display_pending: false,
            keep_changes_box: false,
            display_changed: false,
            two_step: false,
            saved_mode: None,
            display: Display::query(&console.cvars),
        };
        st.reapply(decl, console);
        st
    }

    /// `FUN_14050df80`: re-apply the preset tables to the cvars.
    fn reapply(&mut self, decl: &SectionDecl, console: &mut crate::platform::console::Console) {
        for (index, preset) in [(12, "r_shadowQualityPreset"), (10, "r_oceanQualityPreset"), (9, "r_modelsQualityPreset"), (11, "r_viewDistPreset")] {
            let q = console.cvars.int(preset).unwrap_or(0);
            self.apply_table(decl, console, index, q);
        }
        let aa = console.cvars.int("r_AntiAliasingPreset").unwrap_or(0);
        apply_aa(console, aa);
        for (index, preset) in [(8, "r_textureDetailsPreset"), (13, "r_maxDynamicDecalsPreset")] {
            let q = console.cvars.int(preset).unwrap_or(0);
            self.apply_table(decl, console, index, q);
        }
        let fog = console.cvars.int("r_downsampleQualityPreset").unwrap_or(0);
        self.apply_table(decl, console, FOG_BLUR, fog);
        // A failing `r_mode` lookup selects the default window mode
        // (`FUN_140348bb0`) and `r_renderQualityPresets == -1` selects AUTO;
        // both need the renderer (blockers in docs/flash.md).
    }

    /// `FUN_14050fd30(index, q)`. AUTO needs the CPU/GPU scoring
    /// (`FUN_14050b410` / `FUN_14050b760`), which is not ported, so an AUTO
    /// row is not applied.
    fn apply_table(&mut self, _decl: &SectionDecl, console: &mut crate::platform::console::Console, index: usize, q: i32) {
        if !(0..AUTO).contains(&q) {
            return;
        }
        let q = q as usize;
        let cvars = &mut console.cvars;
        let mut set_float = |name: &str, v: f32| {
            if let Some(i) = cvars.find(name) {
                cvars.set_float(i, v, true);
            }
        };
        match index {
            8 => {
                // `FUN_1405104b0`.
                let size = self.tables.textures[q].max_mip_size;
                let old = console.cvars.int("r_textureStreamingMaxTextureSizeKB").unwrap_or(-1);
                self.texture_cap_lowered = old == -1 || size < old;
                self.texture_lowest = q == 0;
                if let Some(i) = console.cvars.find("r_textureStreamingMaxTextureSizeKB") {
                    console.cvars.set_int(i, size, true);
                }
            }
            9 => {
                // `FUN_14050fb40`.
                let lod = self.tables.lod[q];
                for (k, v) in lod.iter().enumerate() {
                    set_float(&format!("lod_dist_level{k}"), *v);
                }
            }
            10 => {
                // `FUN_140510690`.
                let w = self.tables.water[q].clone();
                set_float("r_oceanTileSize", w.tile_size);
                set_float("r_oceanGeometricDetails", w.geometry_details);
                if let Some(i) = console.cvars.find("r_skipOceanSSR") {
                    console.cvars.set_bool(i, w.skip_ocean_ssr, true);
                }
                if let Some(i) = console.cvars.find("r_water_ScreenSpaceMaxStepsCount") {
                    console.cvars.set_int(i, w.screen_space_max_steps, true);
                }
            }
            11 => {
                // `FUN_1405105a0`.
                set_float("r_lodZoomMultiplicator", self.tables.view_distance[q]);
                set_float("r_umbraGateVisibilityRatio", self.tables.umbra[q]);
            }
            12 => {
                // `FUN_14050ff40`.
                let s = self.tables.shadows[q].clone();
                set_float("r_shadowCullDistance", s.cull_distance);
                let c = &mut console.cvars;
                let mut set = |name: &str, value: String| {
                    if let Some(i) = c.find(name) {
                        c.set(i, Some(&value), true);
                    }
                };
                set("r_shadowParallelPartialDynaUpdate", (s.parallel_partial_dyna_update as i32).to_string());
                set("r_shadowSpotAndOmniPartialUpdateDynaThreshold", float_string(s.spot_omni_part_update_dyna_threshold));
                set(
                    "r_shadowSpotAndOmniPartialUpdateDynaSqrDistanceThreshold",
                    float_string(s.spot_omni_part_update_dyna_sqr_distance_threshold),
                );
                set("r_shadowParallelResolutionDividerLevel", s.parallel_resolution_divider_level.to_string());
                set("r_shadowSpotAndOmniLowResolutionDividerEnabled", (s.spot_omni_low_resolution_divider_enabled as i32).to_string());
                set("r_shadows32Bits", (s.shadows_32_bits as i32).to_string());
                set("r_shadowParallelSliceCnt", s.parallel_slice_cnt.to_string());
                let f = s.cascade_fars;
                set("r_shadowCascadeFars", format!("{:.6} {:.6} {:.6} {:.6}", f[0], f[1], f[2], f[3]));
            }
            13 => {
                // `FUN_14050fad0`.
                if let Some(i) = console.cvars.find("r_maxDynamicDecals") {
                    console.cvars.set_int(i, self.tables.decals[q], true);
                }
            }
            FOG_BLUR => {
                if let Some(i) = console.cvars.find("r_downsampleQuality") {
                    console.cvars.set_int(i, if q != 0 { 1 } else { 2 }, true);
                }
            }
            _ => {}
        }
    }
}

/// `FUN_14050fa10`.
fn apply_aa(console: &mut crate::platform::console::Console, preset: i32) {
    let (skip, mode) = match preset {
        0 => (true, 0),
        1 => (true, 1),
        2 => (true, 2),
        // AUTO (6) is weighted by the CPU/GPU scores, not ported.
        AUTO => return,
        _ => (false, -1),
    };
    let c = &mut console.cvars;
    if let Some(i) = c.find("r_skipTemporalAA") {
        c.set_bool(i, skip, true);
    }
    if let Some(i) = c.find("r_AAMode") {
        c.set_int(i, mode, true);
    }
}

fn state(section: &mut Section) -> &mut State {
    match &mut section.kind {
        Kind::Visuals(s) => s,
        _ => unreachable!("visuals state"),
    }
}

/// `FUN_14050e1a0`: MONITOR and VIDEOCARD also need a widget maximum of at
/// least 1 (more than one entry).
pub fn visible(s: &Setting, index: usize) -> bool {
    !matches!(index, MONITOR | VIDEOCARD) || s.widget.maximum >= 1
}

/// V.2: `(refreshEnabled, updateGameCallback, locked)` per setting.
pub fn element_arguments(env: &Env, index: usize) -> (bool, Option<&'static str>, bool) {
    let int = |name: &str| env.console.cvars.int(name).unwrap_or(0);
    match index {
        RESOLUTION => (true, Some("OnTryResolution"), false),
        MONITOR => (true, Some("OnTryMonitor"), false),
        WINDOWED | SWAPBUFFER => (true, Some("OnTryDisplaySetting"), false),
        GAMMA => (false, None, false),
        VIDEOCARD => (true, Some("OnUpdateVideoCard"), false),
        PRESET => (true, Some("OnUpdatePreset"), false),
        ANTIALIASING => (true, Some("OnUpdateAA"), false),
        AA_SHARPNESS => (true, None, int("r_AntiAliasingPreset") != 3),
        VSYNC => (true, Some("OnUpdateVSync"), false),
        RES_SCALE => (true, Some("OnUpdateResScale"), false),
        RES_SCALE_MANUAL => (true, Some("OnUpdateResScaleManual"), int("rs_enable") == 0),
        RES_SCALE_QUALITY => (true, None, int("rs_enable") == 0),
        FRAMERATE => (true, Some("OnUpdateFramerateLimiter"), int("r_swapInterval") != 0),
        _ => (true, None, false),
    }
}

/// The movie value of a setting by enum (vfunc +0x80), or the cvar-based
/// value when `from_cvars`.
fn as3(env: &mut Env, section: &Section, index: usize) -> Result<i32> {
    match section.element_of(env, index) {
        Some(out) => movie_value(env, section, out, index),
        None => Ok(section.decl.settings[index].widget.selected),
    }
}

/// `FUN_14050c200(from_cvars)`: AA sharpness.
fn aa_sharpness(env: &mut Env, section: &mut Section, from_cvars: bool) -> Result<()> {
    let (selected, aa) = if from_cvars {
        (env.console.cvars.int("r_TAAAmount").unwrap_or(0), env.console.cvars.int("r_AntiAliasingPreset").unwrap_or(0))
    } else {
        (as3(env, section, AA_SHARPNESS)?, as3(env, section, ANTIALIASING)?)
    };
    let w = &mut section.decl.settings[AA_SHARPNESS].widget;
    w.selected = selected;
    w.minimum = 1;
    w.maximum = 20;
    w.interval = 1;
    w.locked = aa != 3;
    Ok(())
}

/// `FUN_14050d180(from_cvars)`: V-sync strings with the refresh rate.
fn vsync(env: &mut Env, section: &mut Section, from_cvars: bool) -> Result<()> {
    let refresh = state(section).display.refresh_rate;
    let mut strings = vec![super::text_of(env, "ui_main_off"), super::text_of(env, "ui_main_on"), super::text_of(env, "ui_vsync_half")];
    if env.console.cvars.int("r_fullscreen") == Some(1) && refresh > 0.0 {
        strings[1].push_str(&format!(" ({}Hz)", refresh as i32));
        strings[2].push_str(&format!(" ({}Hz)", (refresh * 0.5) as i32));
    }
    let selected = if from_cvars { env.console.cvars.int("r_swapInterval").unwrap_or(0) } else { as3(env, section, VSYNC)? };
    let w = &mut section.decl.settings[VSYNC].widget;
    w.runtime_string_values = strings;
    w.selected = selected;
    w.minimum = 0;
    w.maximum = 2;
    Ok(())
}

/// The framerate index of `com_timeScaler` (V.3 step 7).
fn framerate_index(env: &Env) -> i32 {
    let scaler = env.console.cvars.float("com_timeScaler").unwrap_or(0.0);
    ((-30 - (scaler * -30.0) as i32) / 15).clamp(0, 6)
}

/// `FUN_14050c470(from_cvars)`: the framerate limiter.
fn framerate(env: &mut Env, section: &mut Section, from_cvars: bool) -> Result<()> {
    let strings = ["30", "45", "60", "75", "90", "105", "120"].iter().map(|s| s.to_string()).collect();
    let (selected, vsync) = if from_cvars {
        (framerate_index(env), env.console.cvars.int("r_swapInterval").unwrap_or(0))
    } else {
        (as3(env, section, FRAMERATE)?, as3(env, section, VSYNC)?)
    };
    let w = &mut section.decl.settings[FRAMERATE].widget;
    w.runtime_string_values = strings;
    w.selected = selected;
    w.locked = vsync != 0;
    Ok(())
}

/// `FUN_1405127b0(from_cvars)`: the manual resolution-scale limits, then
/// refresh element 26.
fn res_scale_limits(env: &mut Env, section: &mut Section, from_cvars: bool) -> Result<()> {
    let refresh = state(section).display.refresh_rate;
    let (vsync, frame, target, enabled) = if from_cvars {
        (
            env.console.cvars.int("r_swapInterval").unwrap_or(0),
            framerate_index(env),
            env.console.cvars.float("rs_FPSTargeted").unwrap_or(0.0) as i32,
            env.console.cvars.int("rs_enable").unwrap_or(0),
        )
    } else {
        (as3(env, section, VSYNC)?, as3(env, section, FRAMERATE)?, as3(env, section, RES_SCALE_MANUAL)?, as3(env, section, RES_SCALE)?)
    };
    let w = &mut section.decl.settings[RES_SCALE_MANUAL].widget;
    w.maximum = if vsync >= 1 { (refresh / vsync as f32) as i32 } else { frame * 15 + 30 };
    w.selected = target.clamp(w.minimum, w.maximum.max(w.minimum));
    w.locked = w.maximum == w.minimum || enabled == 0;
    refresh_index(env, section, RES_SCALE_MANUAL)
}

fn refresh_index(env: &mut Env, section: &Section, index: usize) -> Result<()> {
    if section.filled {
        refresh(env, section, index)
    } else {
        Ok(())
    }
}

/// `FUN_14050f670` (`OnUpdateResScale`).
fn res_scale_mode(env: &mut Env, section: &mut Section) -> Result<()> {
    let mode = as3(env, section, RES_SCALE)?;
    let refresh = state(section).display.refresh_rate;
    let w = section.decl.settings[RES_SCALE_MANUAL].widget.clone();
    let manual = match mode {
        2 => as3(env, section, RES_SCALE_MANUAL)?,
        3 => w.maximum,
        1 => {
            let v = as3(env, section, VSYNC)?;
            (if v > 0 { refresh / v as f32 } else { refresh }) as i32
        }
        _ => w.selected,
    };
    let quality = as3(env, section, RES_SCALE_QUALITY)?;
    let s = &mut section.decl.settings;
    s[RES_SCALE_MANUAL].widget.selected = manual.clamp(w.minimum, w.maximum.max(w.minimum));
    s[RES_SCALE_MANUAL].widget.locked = mode == 0;
    s[RES_SCALE_QUALITY].widget.locked = mode == 0;
    s[RES_SCALE_QUALITY].widget.selected = quality;
    refresh_index(env, section, RES_SCALE_QUALITY)?;
    refresh_index(env, section, RES_SCALE_MANUAL)
}

/// `FUN_14050d130` (PreFill): runtime lists and labels. The monitor and
/// resolution fills ask the renderer each time (`FUN_14050cd00`: vfunc
/// `+0x2f0` for the menu modes, `+0x278` for `r_mode`'s mode), so an applied
/// mode shows when the section is filled again.
pub fn pre_fill(env: &mut Env, section: &mut Section) -> Result<()> {
    state(section).display = Display::query(&env.console.cvars);
    let display = state(section).display.clone();
    if section.decl.settings[MONITOR].runtime_setting {
        // `FUN_14050c920`.
        let w = &mut section.decl.settings[MONITOR].widget;
        w.runtime_string_values = display.monitors.iter().enumerate().map(|(i, n)| format!("{} . {n}", i + 1)).collect();
        w.minimum = 0;
        w.maximum = display.monitors.len() as i32 - 1;
        w.interval = 1;
        w.selected = display.current_monitor;
    }
    if section.decl.settings[RESOLUTION].runtime_setting {
        fill_resolution(section, &display);
    }
    if section.decl.settings[VIDEOCARD].runtime_setting {
        // `FUN_14050d600`.
        let selected = env.console.cvars.int("r_d3dAdapter").unwrap_or(0);
        let w = &mut section.decl.settings[VIDEOCARD].widget;
        w.runtime_string_values = display.adapters.iter().map(|a| a.1.clone()).collect();
        w.selected = selected;
        w.minimum = 0;
        w.maximum = display.adapters.len() as i32 - 1;
    }
    if section.decl.settings[HBAO].runtime_setting {
        section.decl.settings[HBAO].runtime_label = "HBAO +".into();
    }
    aa_sharpness(env, section, true)?;
    if section.decl.settings[VSYNC].runtime_setting {
        vsync(env, section, true)?;
    }
    if section.decl.settings[FRAMERATE].runtime_setting {
        framerate(env, section, true)?;
    }
    Ok(())
}

/// `FUN_14050cd00`: the resolution list and the row of `r_mode`'s mode.
fn fill_resolution(section: &mut Section, display: &Display) {
    let w = &mut section.decl.settings[RESOLUTION].widget;
    w.runtime_string_values = display.modes.iter().map(|&(mw, mh)| resolution_label(mw, mh)).collect();
    let current = display.current_mode.map(|m| (m.0, m.1));
    w.selected = display.modes.iter().position(|&m| Some(m) == current).unwrap_or(0) as i32;
    w.minimum = 0;
    w.maximum = display.modes.len() as i32 - 1;
    w.interval = 1;
}

/// `FUN_14050cd00`'s label: `"%d x %d (%d:%d)"` for small ratios (8:5 shown
/// as 16:10), `"%d x %d"` otherwise.
pub fn resolution_label(w: i32, h: i32) -> String {
    fn gcd(a: i32, b: i32) -> i32 {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let g = gcd(w, h).max(1);
    let (mut a, mut b) = (w / g, h / g);
    if (a, b) == (8, 5) {
        (a, b) = (16, 10);
    }
    if a * b < 271 {
        format!("{w} x {h} ({a}:{b})")
    } else {
        format!("{w} x {h}")
    }
}

/// After FillSettings: the snapshot of all 31 widgets, then the manual
/// resolution-scale limits from the cvars.
pub fn after_fill(env: &mut Env, section: &mut Section) -> Result<()> {
    let values: Vec<i32> = section.decl.settings.iter().map(|s| s.widget.selected).collect();
    state(section).snapshot.copy_from_slice(&values[..SETTINGS]);
    res_scale_limits(env, section, true)
}

/// `FUN_140512dd0`: cvars into the widgets.
pub fn load(env: &mut Env, section: &mut Section, index: usize) {
    let int = |env: &Env, name: &str| env.console.cvars.int(name).unwrap_or(0);
    let selected = match index {
        RESOLUTION => {
            let st = state(section);
            let current = st.display.current_mode.map(|m| (m.0, m.1));
            st.display.modes.iter().position(|&m| Some(m) == current).unwrap_or(0) as i32
        }
        MONITOR => state(section).display.current_monitor,
        WINDOWED => int(env, "r_fullscreen"),
        GAMMA => return load_cvar(env, &mut section.decl.settings[index], "r_gamma", false),
        4 => return load_cvar(env, &mut section.decl.settings[index], "ark_fieldOfView", false),
        5 => return load_cvar(env, &mut section.decl.settings[index], "r_BlackAndWhiteMode", false),
        VIDEOCARD => int(env, "r_d3dAdapter"),
        PRESET => int(env, "r_renderQualityPresets"),
        8 => int(env, "r_textureDetailsPreset"),
        9 => int(env, "r_modelsQualityPreset"),
        10 => int(env, "r_oceanQualityPreset"),
        11 => int(env, "r_viewDistPreset"),
        12 => int(env, "r_shadowQualityPreset"),
        13 => int(env, "r_maxDynamicDecalsPreset"),
        FOG_BLUR => {
            let v = (int(env, "r_downsampleQuality") == 1) as i32;
            let i = env.cvar("r_downsampleQualityPreset");
            env.console.cvars.set_int(i, v, true);
            v
        }
        ANTIALIASING => int(env, "r_AntiAliasingPreset"),
        AA_SHARPNESS => return load_cvar(env, &mut section.decl.settings[index], "r_TAAAmount", false),
        17 => return load_cvar(env, &mut section.decl.settings[index], "r_skipBloom", true),
        18 => return load_cvar(env, &mut section.decl.settings[index], "r_skipCameraMotionBlur", true),
        19 => return load_cvar(env, &mut section.decl.settings[index], "r_skipDofBck", true),
        20 => return load_cvar(env, &mut section.decl.settings[index], "r_skipLensFlares", true),
        21 => return load_cvar(env, &mut section.decl.settings[index], "r_skipGodRays", true),
        22 => return load_cvar(env, &mut section.decl.settings[index], "r_shadowsRats", false),
        23 => return load_cvar(env, &mut section.decl.settings[index], "r_shadowsBloodflies", false),
        VSYNC => int(env, "r_swapInterval"),
        RES_SCALE => {
            if int(env, "rs_enable") == 0 {
                0
            } else if int(env, "rs_VSyncBased") != 0 {
                1
            } else if env.console.cvars.float("rs_FPSTargeted") == Some(f32::MAX) {
                3
            } else {
                2
            }
        }
        RES_SCALE_MANUAL => {
            let w = &section.decl.settings[index].widget;
            let fps = env.console.cvars.float("rs_FPSTargeted").unwrap_or(0.0);
            if fps == f32::MAX { w.maximum } else { (fps as i32).clamp(w.minimum, w.maximum.max(w.minimum)) }
        }
        RES_SCALE_QUALITY => {
            let s = env.console.cvars.float("rs_minimumResolutionScale").unwrap_or(0.0);
            if s > 0.55 {
                if s <= 0.8 { 1 } else { 0 }
            } else {
                2
            }
        }
        FRAMERATE => framerate_index(env),
        SWAPBUFFER => return load_cvar(env, &mut section.decl.settings[index], "r_swapChainTripleBufferEnable", false),
        HBAO => return load_cvar(env, &mut section.decl.settings[index], "r_hbao", false),
        _ => return,
    };
    section.decl.settings[index].widget.selected = selected;
}

/// `FUN_140511a70`: one widget into the game.
fn apply_setting(env: &mut Env, section: &mut Section, index: usize) -> Result<Vec<Effect>> {
    let s = section.decl.settings[index].clone();
    let sel = s.widget.selected;
    let set_int = |env: &mut Env, name: &str, v: i32| {
        let i = env.cvar(name);
        env.console.cvars.set_int(i, v, true);
    };
    match index {
        RESOLUTION | MONITOR => {
            // `FUN_140512990(0)` / the MONITOR case: validate through
            // vfunc +0x190, then `FUN_140345890` writes the window cvars.
            let d = state(section).display.clone();
            let (mon, w, h) = match index {
                RESOLUTION => {
                    let Some(&(w, h)) = d.modes.get(sel.max(0) as usize) else { return Ok(Vec::new()) };
                    (d.current_monitor, w, h)
                }
                _ => {
                    let (w, h, _) = d.current_mode.unwrap_or((1, 1, 0));
                    (sel, w, h)
                }
            };
            if select_mode(env, &d, mon, w, h) {
                state(section).display_pending = true;
                return Ok(vec![Effect::DisplayChanged]);
            }
        }
        WINDOWED => {
            set_int(env, "r_fullscreen", sel);
            state(section).display_pending = true;
            return Ok(vec![Effect::DisplayChanged]);
        }
        GAMMA => apply_cvar(env, &s, "r_gamma", false),
        4 => apply_cvar(env, &s, "ark_fieldOfView", false),
        5 => apply_cvar(env, &s, "r_BlackAndWhiteMode", false),
        VIDEOCARD => {
            set_int(env, "r_d3dAdapter", sel);
            let adapters = state(section).display.adapters.clone();
            if let Some(a) = adapters.get(sel.max(0) as usize).filter(|_| sel >= 0) {
                let i = env.cvar("r_d3dAdapterId");
                env.console.cvars.set(i, Some(&a.0), true);
            }
        }
        PRESET => set_int(env, "r_renderQualityPresets", sel),
        8..=13 => {
            let preset = ["r_textureDetailsPreset", "r_modelsQualityPreset", "r_oceanQualityPreset", "r_viewDistPreset", "r_shadowQualityPreset", "r_maxDynamicDecalsPreset"][index - 8];
            set_int(env, preset, sel);
            let decl = section.decl.clone();
            state(section).apply_table(&decl, env.console, index, sel);
        }
        FOG_BLUR => {
            let (preset, quality) = if sel == 0 { (0, 2) } else { (1, 1) };
            set_int(env, "r_downsampleQualityPreset", preset);
            set_int(env, "r_downsampleQuality", quality);
        }
        ANTIALIASING => {
            set_int(env, "r_AntiAliasingPreset", sel);
            apply_aa(env.console, sel);
        }
        AA_SHARPNESS => set_int(env, "r_TAAAmount", sel),
        17 => apply_cvar(env, &s, "r_skipBloom", true),
        18 => apply_cvar(env, &s, "r_skipCameraMotionBlur", true),
        19 => apply_cvar(env, &s, "r_skipDofBck", true),
        20 => apply_cvar(env, &s, "r_skipLensFlares", true),
        21 => {
            apply_cvar(env, &s, "r_skipGodRays", true);
            apply_cvar(env, &s, "r_skipGodRaysUnderWater", true);
        }
        22 => apply_cvar(env, &s, "r_shadowsRats", false),
        23 => apply_cvar(env, &s, "r_shadowsBloodflies", false),
        VSYNC => set_int(env, "r_swapInterval", sel),
        RES_SCALE => {
            // `FUN_140512ac0(0)`.
            if sel == 0 {
                set_int(env, "rs_enable", 0);
            } else {
                set_int(env, "rs_enable", 3);
                match sel {
                    1 => set_int(env, "rs_VSyncBased", 1),
                    2 => {
                        let manual = as3(env, section, RES_SCALE_MANUAL)?;
                        let i = env.cvar("rs_FPSTargeted");
                        env.console.cvars.set_float(i, manual as f32, true);
                        set_int(env, "rs_VSyncBased", 0);
                    }
                    _ => {
                        let i = env.cvar("rs_FPSTargeted");
                        env.console.cvars.set_float(i, f32::MAX, true);
                        set_int(env, "rs_VSyncBased", 0);
                    }
                }
            }
        }
        RES_SCALE_MANUAL => {
            let v = if as3(env, section, RES_SCALE)? == 3 { f32::MAX } else { sel as f32 };
            let i = env.cvar("rs_FPSTargeted");
            env.console.cvars.set_float(i, v, true);
        }
        RES_SCALE_QUALITY => {
            let i = env.cvar("rs_minimumResolutionScale");
            env.console.cvars.set_float(i, [0.85, 0.75, 0.5][sel.clamp(0, 2) as usize], true);
        }
        FRAMERATE => {
            if as3(env, section, VSYNC)? == 0 {
                let i = env.cvar("com_timeScaler");
                env.console.cvars.set_float(i, (sel * 15 + 30) as f32 / 30.0, true);
            }
        }
        SWAPBUFFER => apply_cvar(env, &s, "r_swapChainTripleBufferEnable", false),
        HBAO => apply_cvar(env, &s, "r_hbao", false),
        _ => {}
    }
    Ok(Vec::new())
}

/// `FUN_14050dd60`: any visible setting whose movie value differs from the
/// fill snapshot.
pub fn has_changes(env: &mut Env, section: &mut Section) -> Result<bool> {
    for (out, index) in section.elements(env) {
        let v = match section.decl.settings[index].widget.kind {
            widget::GAMMA => env.get_int(&format!("{CURRENT}.settings.data[{out}].link.gameDataProvider.data[0].slider.value"))?,
            _ => movie_value(env, section, out, index)?,
        };
        if v != state(section).snapshot[index] {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `FUN_1405120d0`.
pub fn apply(env: &mut Env, section: &mut Section) -> Result<bool> {
    let mut any = false;
    for (out, index) in section.elements(env) {
        section.decl.settings[index].widget.selected = state(section).snapshot[index];
        if read_back(env, section, out, index, true)? {
            any = true;
            apply_setting(env, section, index)?;
        }
    }
    if env.main_menu_active {
        let dof = env.console.cvars.int("r_skipDofBck").unwrap_or(0) != 0;
        let i = env.cvar("r_skipDOF");
        env.console.cvars.set_bool(i, dof, true);
    }
    Ok(any)
}

/// `FUN_14050f850`. The default window mode (`FUN_140348bb0`) and the AUTO
/// preset need the renderer and the hardware scoring (blockers).
pub fn restore(env: &mut Env, section: &mut Section) -> Result<()> {
    for name in [
        "r_swapChainTripleBufferEnable",
        "com_disableFrameSpinning",
        "com_timeScaler",
        "r_fullscreen",
        "r_swapInterval",
        "rs_VSyncBased",
        "r_TAAAmount",
        "rs_FPSTargeted",
    ] {
        let i = env.cvar(name);
        env.console.cvars.reset(i);
    }
    let st = state(section);
    st.display_pending = true;
    st.keep_changes_box = true;
    let i = env.cvar("r_d3dAdapter");
    env.console.cvars.set_int(i, 0, true);
    for name in ["r_d3dAdapterId", "rs_enable", "rs_minimumResolutionScale"] {
        let i = env.cvar(name);
        env.console.cvars.reset(i);
    }
    let i = env.cvar("r_renderQualityPresets");
    env.console.cvars.set_int(i, AUTO, true);
    for name in ["ark_fieldOfView", "r_hbao"] {
        let i = env.cvar(name);
        env.console.cvars.reset(i);
    }
    state(section).two_step = true;
    for name in ["r_gamma", "r_BlackAndWhiteMode"] {
        let i = env.cvar(name);
        env.console.cvars.reset(i);
    }
    if env.main_menu_active {
        let dof = env.console.cvars.int("r_skipDofBck").unwrap_or(0) != 0;
        let i = env.cvar("r_skipDOF");
        env.console.cvars.set_bool(i, dof, true);
    }
    Ok(())
}

/// `FUN_14050e2a0` before the base Close: without apply, the snapshot goes
/// back into the widgets and the previews are reverted. Returns whether the
/// config must be saved (`+0x30a`).
pub fn close(env: &mut Env, section: &mut Section, apply_changes: bool) -> Result<bool> {
    if !apply_changes {
        let snapshot = state(section).snapshot;
        for (i, s) in section.decl.settings.iter_mut().enumerate() {
            s.widget.selected = snapshot[i];
        }
        for (_, index) in section.elements(env) {
            refresh(env, section, index)?;
        }
        for index in (3..SETTINGS).filter(|&i| i != SWAPBUFFER) {
            if section.visible(env, index) {
                apply_setting(env, section, index)?;
            }
        }
    }
    let st = state(section);
    let save = st.display_changed;
    st.display_changed = false;
    st.display_pending = false;
    // With apply the texture streamer is told about lowered caps
    // (`FUN_14030cec0` / `FUN_14030d0f0`); the rewrite has no streamer.
    st.texture_lowest = false;
    st.texture_cap_lowered = false;
    Ok(save)
}

fn gamma_value(env: &Env, v: i32) -> f32 {
    let c = &env.console.cvars.list[env.cvar("r_gamma")];
    c.min + v as f32 * (4.0 - 0.5) / 35.0
}

fn set_gamma(env: &mut Env, v: i32) {
    let value = gamma_value(env, v);
    let i = env.cvar("r_gamma");
    env.console.cvars.set_float(i, value, true);
}

/// `FUN_140510880`: the gamma link.
fn fill_gamma(env: &mut Env, section: &mut Section, out: usize, value: i32) -> Result<()> {
    let link = format!("{CURRENT}.settings.data[{out}].link");
    env.set(&format!("{link}.currentValue"), (value as f64).into())?;
    env.set(&format!("{link}.as3callback"), Value::from("_C_DisplayGammaSettings"))?;
    if env.profile_int("HasSetGamma") != 0 {
        env.set(&format!("{link}.gameCallbackConfirmation"), Value::from("OnGammaModifiedConfirmation"))?;
        env.set(&format!("{link}.gameCallbackCancel"), Value::from("OnGammaCancel"))?;
    }
    env.set(&format!("{link}.gameDataProvider.length"), 1.0.into())?;
    let label = env.text(section.decl.settings[GAMMA].widget.description);
    env.set(&format!("{link}.gameDataProvider.data[0].label"), Value::from(label.as_str()))?;
    let slider = format!("{link}.gameDataProvider.data[0].slider");
    env.set(&format!("{slider}.value"), (value as f64).into())?;
    env.set(&format!("{slider}.minimum"), 0.0.into())?;
    env.set(&format!("{slider}.maximum"), 35.0.into())?;
    env.set(&format!("{slider}.interval"), 1.0.into())?;
    env.set(&format!("{slider}.displayValue"), false.into())?;
    env.set(&format!("{slider}.updateGameCallback"), Value::from("OnGammaChanged"))
}

/// Vfunc +0x90 (`FUN_14050c1d0`).
pub fn link_fill(env: &mut Env, section: &mut Section, out: usize, index: usize) -> Result<()> {
    let value = section.decl.settings[index].widget.selected;
    state(section).gamma = value;
    fill_gamma(env, section, out, value)
}

/// The gamma element (vfunc +0xa8): the visible count among
/// RESOLUTION/MONITOR/WINDOWED.
fn gamma_element(env: &Env, section: &Section) -> usize {
    [RESOLUTION, MONITOR, WINDOWED].iter().filter(|&&i| section.visible(env, i)).count()
}

fn gamma_slider_value(env: &mut Env, section: &Section) -> Result<i32> {
    let out = gamma_element(env, section);
    env.get_int(&format!("{CURRENT}.settings.data[{out}].link.gameDataProvider.data[0].slider.value"))
}

/// Vfunc +0x98 / +0xa0.
pub fn link_read(env: &mut Env, section: &mut Section, index: usize, store: bool) -> Result<bool> {
    let v = gamma_slider_value(env, section)?;
    let w = &mut section.decl.settings[index].widget;
    let changed = w.selected != v;
    if store {
        w.selected = v;
    }
    Ok(changed)
}

const GAMMA_SKIPS: [&str; 4] = ["r_skipColorGrading", "r_skipBloom", "r_skipLensFlares", "r_skipToneMapping"];

/// `FUN_1405110d0`: the saved skip cvars back (the gamma FX entity and the
/// pause FX are not hosted yet).
fn end_gamma(env: &mut Env, section: &mut Section) {
    if let Some(saved) = state(section).saved_skips.take() {
        for (name, v) in GAMMA_SKIPS.iter().zip(saved) {
            let i = env.cvar(name);
            env.console.cvars.set_bool(i, v, true);
        }
    }
}

/// `FUN_14050eea0` -> `FUN_1405122d0`: apply a quality preset to the
/// dependent settings.
fn apply_preset(env: &mut Env, section: &mut Section) -> Result<()> {
    let q = env.console.cvars.int("r_renderQualityPresets").unwrap_or(0);
    if q == AUTO {
        // `FUN_14050ace0` needs the CPU/GPU scoring (blocker).
        return Ok(());
    }
    let decl = section.decl.clone();
    for (index, preset) in [
        (12, "r_shadowQualityPreset"),
        (10, "r_oceanQualityPreset"),
        (9, "r_modelsQualityPreset"),
        (8, "r_textureDetailsPreset"),
        (13, "r_maxDynamicDecalsPreset"),
        (11, "r_viewDistPreset"),
    ] {
        let i = env.cvar(preset);
        env.console.cvars.set_int(i, q, true);
        state(section).apply_table(&decl, env.console, index, q);
        section.decl.settings[index].widget.selected = q;
        refresh_index(env, section, index)?;
    }
    let aa = match q {
        0 => 1,
        1 => 2,
        _ => 3,
    };
    let i = env.cvar("r_AntiAliasingPreset");
    env.console.cvars.set_int(i, aa, true);
    apply_aa(env.console, aa);
    section.decl.settings[ANTIALIASING].widget.selected = aa;
    refresh_index(env, section, ANTIALIASING)?;
    aa_sharpness(env, section, false)?;
    refresh_index(env, section, AA_SHARPNESS)?;
    let i = env.cvar("r_downsampleQualityPreset");
    env.console.cvars.set_int(i, if q > 1 { 3 } else { 1 }, true);
    let i = env.cvar("r_downsampleQuality");
    env.console.cvars.set_int(i, 1, true);
    load(env, section, FOG_BLUR);
    refresh_index(env, section, FOG_BLUR)?;
    let low = q < 3;
    for name in ["r_skipBloom", "r_skipCameraMotionBlur", "r_skipLensFlares", "r_skipGodRays", "r_skipGodRaysUnderWater", "r_skipDofBck"] {
        let i = env.cvar(name);
        env.console.cvars.set_bool(i, low, true);
    }
    for name in ["r_shadowsRats", "r_shadowsBloodflies"] {
        let i = env.cvar(name);
        env.console.cvars.set_bool(i, !low, true);
    }
    for index in 17..=23 {
        section.decl.settings[index].widget.selected = (!low) as i32;
        refresh_index(env, section, index)?;
    }
    Ok(())
}

/// Vfunc +0x190 then `FUN_140345890`: `r_mode` is always written, the
/// window cvars when the mode changed. Returns the validation result.
fn select_mode(env: &mut Env, d: &Display, mon: i32, w: i32, h: i32) -> bool {
    let old = env.console.cvars.int("r_mode").unwrap_or(-1);
    let (r_mode, w, h, ok) = d.table.select(old, mon, w, h);
    let i = env.cvar("r_mode");
    env.console.cvars.set_int(i, r_mode, true);
    if ok {
        if let Some((w, h, x, y)) = d.table.window(mon, w, h) {
            for (name, v) in [("r_windowWidth", w), ("r_windowHeight", h), ("r_windowPosX", x), ("r_windowPosY", y)] {
                let i = env.cvar(name);
                env.console.cvars.set_int(i, v, true);
            }
        }
    }
    ok
}

/// `FUN_140511250(a, b)`: try the display settings in the movie.
fn try_display(env: &mut Env, section: &mut Section, two_step: bool) -> Result<Vec<Effect>> {
    let d = state(section).display.clone();
    let fullscreen = env.console.cvars.int("r_fullscreen").unwrap_or(0);
    let old_triple = env.console.cvars.int("r_swapChainTripleBufferEnable").unwrap_or(0) != 0;
    let (w, h, mon) = d.current_mode.unwrap_or((1, 1, 0));
    state(section).saved_mode = Some((w, h, mon, fullscreen, old_triple));
    let triple = as3(env, section, SWAPBUFFER)? != 0;
    let i = env.cvar("r_swapChainTripleBufferEnable");
    env.console.cvars.set_bool(i, triple, true);
    let windowed = as3(env, section, WINDOWED)?;
    let monitor = as3(env, section, MONITOR)?;
    let (rw, rh) = d.modes.get(as3(env, section, RESOLUTION)?.max(0) as usize).copied().unwrap_or((w, h));
    state(section).two_step = two_step;
    let i = env.cvar("r_fullscreen");
    env.console.cvars.set_int(i, windowed, true);
    let mode_changed = select_mode(env, &d, monitor, rw, rh);
    if mode_changed || windowed != fullscreen || triple != old_triple {
        let st = state(section);
        st.display_pending = true;
        st.keep_changes_box = true;
        return Ok(vec![Effect::DisplayChanged]);
    }
    Ok(Vec::new())
}

/// `FUN_14050f120(index, v)`: a widget's selected value.
fn set_selected(section: &mut Section, index: usize, v: i32) {
    section.decl.settings[index].widget.selected = v;
}

fn set_fullscreen(env: &mut Env, v: i32) {
    let i = env.cvar("r_fullscreen");
    env.console.cvars.set_int(i, v, true);
}

/// `FUN_140512990(widget, 1)`: the resolution row of `r_mode`'s mode
/// (vfunc +0x278); unchanged when the list lacks it, row 0 when `r_mode`
/// names no mode. Then refresh element 0.
fn select_current_resolution(env: &mut Env, section: &mut Section) -> Result<()> {
    let d = state(section).display.clone();
    if !d.modes.is_empty() {
        match d.current_mode {
            Some((w, h, _)) => {
                if let Some(row) = d.modes.iter().position(|&m| m == (w, h)) {
                    set_selected(section, RESOLUTION, row as i32);
                }
            }
            None => set_selected(section, RESOLUTION, 0),
        }
    }
    refresh_index(env, section, RESOLUTION)
}

/// `ui_keep_changes`' timeout (`FUN_140510d70`, `DAT_141b1a400`).
const KEEP_CHANGES_SECONDS: f64 = 13.0;

/// `FUN_140511410` (vfunc +0x28): per-frame Update. A pending display
/// change is finished: the renderer applies the mode (`vfunc +0x200`,
/// `FUN_1400d1a90`; here the host window follows the cvars, so the two-step
/// pass through borderless lands as its last value) and the display widgets
/// are read back. The next frame shows the keep-changes box.
///
/// Not ported: the window-focus gate (`DAT_14230a230`), the external
/// fullscreen toggle (`FUN_140346be0` / `DAT_143982ce0`), the swap-chain
/// recreation for a triple-buffer change (`FUN_14036aa40`) and the window
/// placement before the box (`FUN_140347630`); they belong to the host window.
pub fn update(env: &mut Env, section: &mut Section) -> Result<Vec<Effect>> {
    let st = state(section);
    if !st.display_pending {
        if !std::mem::take(&mut st.keep_changes_box) {
            return Ok(Vec::new());
        }
        // `FUN_140510d70`: buttons [no, yes]; the movie answers 0 at the timeout.
        return Ok(vec![Effect::MessageBox(MessageBox { text: "ui_keep_changes", buttons: 2, timer: KEEP_CHANGES_SECONDS })]);
    }
    let two_step = st.two_step;
    let saved_fullscreen = st.saved_mode.map(|m| m.3);
    if two_step {
        set_selected(section, WINDOWED, 2);
        refresh_index(env, section, WINDOWED)?;
        set_fullscreen(env, 2);
        if saved_fullscreen == Some(1) {
            set_selected(section, WINDOWED, 1);
            refresh_index(env, section, WINDOWED)?;
            set_fullscreen(env, 1);
        }
    }
    let display = Display::query(&env.console.cvars);
    state(section).display = display.clone();
    set_selected(section, MONITOR, display.current_monitor);
    refresh_index(env, section, MONITOR)?;
    fill_resolution(section, &display);
    refresh_index(env, section, RESOLUTION)?;
    select_current_resolution(env, section)?;
    let fullscreen = env.console.cvars.int("r_fullscreen").unwrap_or(0);
    set_selected(section, WINDOWED, fullscreen);
    refresh_index(env, section, WINDOWED)?;
    let triple = env.console.cvars.int("r_swapChainTripleBufferEnable").unwrap_or(0) != 0;
    set_selected(section, SWAPBUFFER, triple as i32);
    refresh_index(env, section, SWAPBUFFER)?;
    vsync(env, section, false)?;
    refresh_index(env, section, VSYNC)?;
    res_scale_limits(env, section, false)?;
    res_scale_mode(env, section)?;
    let mut values = [0; 4];
    for (v, index) in values.iter_mut().zip([MONITOR, RESOLUTION, WINDOWED, SWAPBUFFER]) {
        *v = as3(env, section, index)?;
    }
    let st = state(section);
    st.display_pending = false;
    st.display_changed = true;
    for (v, index) in values.into_iter().zip([MONITOR, RESOLUTION, WINDOWED, SWAPBUFFER]) {
        st.snapshot[index] = v;
    }
    Ok(Vec::new())
}

/// `FUN_14050eaf0`: the keep-changes answer. 0 (No, or the timeout) puts
/// back the mode saved by the try and applies it again without a new box.
pub fn keep_changes(env: &mut Env, section: &mut Section, answer: u32) -> Result<Vec<Effect>> {
    let mut effects = Vec::new();
    let saved = state(section).saved_mode;
    let mut two_step = false;
    if answer == 0 {
        if let Some((w, h, mon, fullscreen, triple)) = saved {
            let i = env.cvar("r_swapChainTripleBufferEnable");
            env.console.cvars.set_bool(i, triple, true);
            set_fullscreen(env, fullscreen);
            let d = state(section).display.clone();
            select_mode(env, &d, mon, w, h);
            state(section).display_pending = true;
            effects.push(Effect::DisplayChanged);
            if state(section).two_step && fullscreen == 1 {
                set_selected(section, WINDOWED, 2);
                refresh_index(env, section, WINDOWED)?;
                set_fullscreen(env, 2);
                two_step = true;
            }
        }
    }
    let st = state(section);
    st.two_step = two_step;
    // The saved mode is cleared; with the two-step pass its fullscreen value stays.
    st.saved_mode = saved.filter(|_| two_step).map(|m| (-1, -1, -1, m.3, false));
    Ok(effects)
}

pub fn handle(env: &mut Env, section: &mut Section, name: &str, args: &[Value]) -> Option<Result<Vec<Effect>>> {
    const NAMES: [&str; 16] = [
        "OnGammaOpening",
        "OnGammaChanged",
        "OnGammaCancel",
        "OnGammaModifiedConfirmation",
        "OnStandaloneGammaCancel",
        "OnStandaloneGammaModifiedConfirmation",
        "OnUpdatePreset",
        "OnUpdateVideoCard",
        "OnTryDisplaySetting",
        "OnTryMonitor",
        "OnTryResolution",
        "OnUpdateVSync",
        "OnUpdateResScale",
        "OnUpdateResScaleManual",
        "OnUpdateFramerateLimiter",
        "OnUpdateAA",
    ];
    if !NAMES.contains(&name) {
        return None;
    }
    let arg = args.first().cloned().unwrap_or(Value::Undefined);
    Some((|| -> Result<Vec<Effect>> {
        let fullscreen = env.console.cvars.int("r_fullscreen").unwrap_or(0) != 0;
        let member = |env: &mut Env, name: &str| -> Result<i32> {
            Ok(env.player.member(&arg, name).map_err(|e| env.player.describe(&e))?.primitive_to_number() as i32)
        };
        match name {
            // `FUN_14050ea20`.
            "OnGammaOpening" => {
                let saved = GAMMA_SKIPS.map(|n| env.console.cvars.int(n).unwrap_or(0) != 0);
                state(section).saved_skips = Some(saved);
                for n in GAMMA_SKIPS {
                    let i = env.cvar(n);
                    env.console.cvars.set_bool(i, true, true);
                }
                let g = state(section).gamma;
                set_gamma(env, g);
            }
            "OnGammaChanged" => {
                let v = member(env, "value")?;
                set_gamma(env, v);
            }
            "OnGammaCancel" | "OnStandaloneGammaCancel" => {
                let out = gamma_element(env, section);
                let g = state(section).gamma;
                fill_gamma(env, section, out, g)?;
                let selected = section.decl.settings[GAMMA].widget.selected;
                set_gamma(env, selected);
                end_gamma(env, section);
                env.call_root("_C_BackFromOptionSection")?;
            }
            "OnGammaModifiedConfirmation" => {
                let v = gamma_slider_value(env, section)?;
                state(section).gamma = v;
                let out = gamma_element(env, section);
                fill_gamma(env, section, out, v)?;
                let selected = section.decl.settings[GAMMA].widget.selected;
                set_gamma(env, selected);
                end_gamma(env, section);
                env.call_root("_C_BackFromOptionSection")?;
            }
            "OnStandaloneGammaModifiedConfirmation" => {
                let v = gamma_slider_value(env, section)?;
                state(section).gamma = v;
                set_gamma(env, v);
                end_gamma(env, section);
                env.call_root("_C_BackFromOptionSection")?;
            }
            "OnUpdatePreset" => {
                let v = member(env, "selectedIndex")?;
                let i = env.cvar("r_renderQualityPresets");
                env.console.cvars.set_int(i, v, true);
                apply_preset(env, section)?;
            }
            "OnUpdateVideoCard" => {
                let st = state(section);
                if !st.video_card_box_shown {
                    st.video_card_box_shown = true;
                    return Ok(vec![Effect::MessageBox(MessageBox { text: "ui_change_effect", buttons: 0, timer: 0.0 })]);
                }
            }
            "OnTryDisplaySetting" => return try_display(env, section, false),
            "OnTryMonitor" => return try_display(env, section, fullscreen),
            "OnTryResolution" => return try_display(env, section, fullscreen),
            "OnUpdateVSync" => {
                framerate(env, section, false)?;
                refresh_index(env, section, FRAMERATE)?;
                res_scale_limits(env, section, false)?;
                res_scale_mode(env, section)?;
            }
            "OnUpdateResScale" => res_scale_mode(env, section)?,
            "OnUpdateResScaleManual" => {
                // `FUN_14050f5d0`: show RES_SCALE as manual, keep its value.
                let kept = section.decl.settings[RES_SCALE].widget.selected;
                section.decl.settings[RES_SCALE].widget.selected = 2;
                refresh_index(env, section, RES_SCALE)?;
                section.decl.settings[RES_SCALE].widget.selected = kept;
            }
            "OnUpdateFramerateLimiter" => res_scale_limits(env, section, false)?,
            _ => {
                aa_sharpness(env, section, false)?;
                refresh_index(env, section, AA_SHARPNESS)?;
            }
        }
        Ok(Vec::new())
    })())
}
