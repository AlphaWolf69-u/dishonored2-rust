//! World-authored load fades: 1408aac09/1f select the initial/later definition;
//! 140166f60 starts it, 140166e40 advances elapsed seconds, 140167170 samples.
//! Host: the existing display UI layer blends the fullscreen colour, without
//! another post-process target or a permanent gameplay pass.
use super::archive::Catalog;
use crate::{
    entities::{Document, Value},
    gameplay::options::decl::{atom, scope},
};
use bevy::prelude::*;
struct Definition {
    fade: std::sync::Arc<crate::assets::fx::fade::Fade>,
    duration: f32,
}
#[derive(Resource, Default)]
pub(super) struct LoadingFade {
    definition: Option<Definition>,
    elapsed: Option<f32>,
}
#[derive(Component)]
pub(super) struct FadeLayer;
impl LoadingFade {
    pub fn load(catalog: &Catalog, document: Option<&Document>, initial: bool) -> Result<Self, String> {
        fn find(fields: &[(String, Value)]) -> Option<&Vec<(String, Value)>> {
            for (name, value) in fields {
                if let Value::Scope(fields) = value {
                    if name == "m_fxWorldParameters" {
                        return Some(fields);
                    }
                    if let Some(found) = find(fields) {
                        return Some(found);
                    }
                }
            }
            None
        }
        let Some(fields) =
            document.and_then(|d| d.definitions.iter().find_map(|d| find(&d.fields)))
        else {
            return Ok(Self::default());
        };
        let field = if initial { "m_fadeIn" } else { "m_fadeIn_NotInitialLoad" };
        let Some(fields) = scope(fields, field) else { return Ok(Self::default()) };
        let Some(path) = atom(fields, "m_decl").filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL")) else {
            return Ok(Self::default());
        };
        let duration = atom(fields, "m_duration")
            .ok_or_else(|| format!("fade {path} has no resolved duration"))?
            .parse::<f32>()
            .map_err(|error| error.to_string())?;
        let fade = crate::assets::fx::fade::acquire(catalog, &format!("generated/decls/fade/{path}.decl"))?;
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("LOAD fade definition: {path} duration={duration} opacity_keys={} opacity_values={} initial={initial}",
                fade.opacity.times.len(), fade.opacity.values.len());
        }
        Ok(Self { definition: Some(Definition { fade, duration }), elapsed: None })
    }
    pub fn start(&mut self) {
        self.elapsed = Some(0.);
    }
}
pub(super) fn advance_fade(
    time: Res<Time>,
    paused: Res<crate::gameplay::game_clock_runtime::GamePaused>,
    mut fade: Option<ResMut<LoadingFade>>,
    mut layer: Query<&mut BackgroundColor, With<FadeLayer>>,
    fx: Option<Res<crate::gameplay::effects::screen_fade::FxScreenFadeFrame>>,
) {
    let Some(fade) = fade.as_deref_mut() else {
        return;
    };
    let Some(elapsed) = fade.elapsed else {
        // 14022a6b0 -> 140167030: the registered FX screen fades set the
        // view's fade object colour; with none registered it keeps its last.
        if let Some(rgba) = fx.and_then(|fx| fx.0) {
            for mut colour in &mut layer {
                colour.0 = Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]);
            }
        }
        return;
    };
    if paused.0 {
        return;
    }
    let elapsed = elapsed + time.delta_secs();
    let (rgba, done) = if let Some(definition) = fade.definition.as_ref() {
        let normalized = elapsed / definition.duration.max(0.001);
        (
            definition
                .fade
                .rgba(normalized)
                .expect("validated loading fade"),
            normalized > 1.,
        )
    } else {
        ([0.; 4], true)
    };
    for mut colour in &mut layer {
        colour.0 = Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]);
    }
    fade.elapsed = (!done).then_some(elapsed);
    if done {
        fade.definition = None;
        super::load_timing::mark("loading fade complete");
    }
}
