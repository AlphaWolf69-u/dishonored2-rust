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
    fade: crate::assets::fx::fade::Fade,
    duration: f32,
}
#[derive(Resource, Default)]
pub(super) struct LoadingFade {
    initial: Option<Definition>,
    later: Option<Definition>,
    active: Option<bool>,
    elapsed: f32,
}
#[derive(Component)]
pub(super) struct FadeLayer;
impl LoadingFade {
    pub fn load(catalog: &Catalog, document: Option<&Document>) -> Result<Self, String> {
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
        let read = |name| -> Result<Option<Definition>, String> {
            let Some(fields) = scope(fields, name) else {
                return Ok(None);
            };
            let Some(path) =
                atom(fields, "m_decl").filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL"))
            else {
                return Ok(None);
            };
            let duration = atom(fields, "m_duration")
                .ok_or_else(|| format!("fade {path} has no resolved duration"))?
                .parse::<f32>()
                .map_err(|e| e.to_string())?;
            let source = catalog
                .read(&format!("generated/decls/fade/{path}.decl"))
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("missing fade {path}"))?;
            let fade = crate::assets::fx::fade::Fade::parse(&crate::entities::declaration_text(
                    &source,
                ))?;
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("LOAD fade definition: {path} duration={duration} opacity_keys={} opacity_values={}",
                    fade.opacity.times.len(), fade.opacity.values.len());
            }
            Ok(Some(Definition {
                fade,
                duration,
            }))
        };
        Ok(Self {
            initial: read("m_fadeIn")?,
            later: read("m_fadeIn_NotInitialLoad")?,
            ..Default::default()
        })
    }
    pub fn start(&mut self, initial: bool) {
        self.active = Some(initial);
        self.elapsed = 0.;
    }
}
pub(super) fn advance_fade(
    time: Res<Time>,
    paused: Res<crate::gameplay::game_clock_runtime::GamePaused>,
    mut fade: Option<ResMut<LoadingFade>>,
    mut layer: Query<&mut BackgroundColor, With<FadeLayer>>,
) {
    let Some(fade) = fade.as_deref_mut() else {
        return;
    };
    let Some(initial) = fade.active else { return };
    if paused.0 {
        return;
    }
    fade.elapsed += time.delta_secs();
    let definition = if initial {
        fade.initial.as_ref()
    } else {
        fade.later.as_ref()
    };
    let (rgba, done) = if let Some(definition) = definition {
        let normalized = fade.elapsed / definition.duration.max(0.001);
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
    if done {
        fade.active = None;
        super::load_timing::mark("loading fade complete");
    }
}
