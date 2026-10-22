//! 88e220's special lens-dirt image publisher. Its base transitions do not
//! interpolate from the last displayed image as ordinary numeric tracks do.
use super::environment::MIN_BLEND_DURATION;
use super::environment::{AreaEnvironment, EnvironmentSource};
use crate::{
    archive::Catalog, assets::environment::ParameterReader, entities::EnvironmentRenderParameters,
};

/// Immutable values prepared once from registered defaults and authored layers.
/// `T` can be a resident renderer image handle or an audited resource identity.
pub struct PreparedLensDirt<T> {
    pub registered_default: T,
    world: Option<T>,
    areas: Vec<(String, Option<T>)>,
}
impl<T: Clone> PreparedLensDirt<T> {
    pub fn load(
        catalog: &Catalog,
        world: &EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
        mut image: impl FnMut(&str) -> Result<T, String>,
    ) -> Result<Self, String> {
        const NAME: &str = "env/bloom/lensdirt";
        let empty = EnvironmentRenderParameters { layers: Vec::new() };
        let defaults = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let registered_default = image(&defaults.image_source(NAME)?)?;
        let mut load = |parameters: &EnvironmentRenderParameters| -> Result<Option<T>, String> {
            if parameters.parameter(NAME).is_none() {
                return Ok(None);
            }
            let read = ParameterReader {
                catalog,
                parameters,
            };
            image(&read.image_source(NAME)?).map(Some)
        };
        let world = load(world)?;
        let areas = areas
            .iter()
            .map(|area| Ok((area.id.clone(), load(&area.parameters)?)))
            .collect::<Result<_, String>>()?;
        Ok(Self {
            registered_default,
            world,
            areas,
        })
    }

    /// Area ordering and containment come from the shared native selection
    /// owner. The secondary provider has priority after all area assignments.
    /// No declaration, archive or image work occurs during this update.
    pub fn update(
        &self,
        track: &mut LensDirtTrack<T>,
        areas: &[AreaEnvironment],
        position: [f32; 3],
        world_duration: f32,
        delta: f32,
        parameters_initialized: bool,
        secondary: Option<(usize, T)>,
        overlays: &[(T, f32)],
    ) -> Result<(), String> {
        if areas.len() != self.areas.len()
            || areas
                .iter()
                .zip(&self.areas)
                .any(|(area, (id, _))| &area.id != id)
        {
            return Err("lens-dirt areas changed without preparing their inputs".into());
        }
        let mut selected = self
            .world
            .clone()
            .map(|value| (EnvironmentSource::World, value));
        let mut duration = world_duration;
        for (index, (area, (_, value))) in areas.iter().zip(&self.areas).enumerate() {
            if let Some(value) = value.as_ref().filter(|_| area.contains(position)) {
                selected = Some((EnvironmentSource::Area(index), value.clone()));
                duration = area.blend_duration;
            }
        }
        if let Some((id, value)) = secondary {
            selected = Some((EnvironmentSource::Secondary(id), value));
            duration = 0.0;
        }
        track.update(selected, duration, delta, parameters_initialized);
        for (image, weight) in overlays {
            track.apply_overlay(image.clone(), *weight);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LensDirtBlend<T> {
    pub from: T,
    pub to: T,
    pub weight: f32,
}
impl<T: Clone> LensDirtBlend<T> {
    pub fn apply_overlay(&mut self, target: T, weight: f32) {
        if self.weight >= 0.5 {
            self.from = self.to.clone();
        }
        self.to = target;
        self.weight = weight;
    }
}
struct Transition<T> {
    source: Option<EnvironmentSource>,
    target: T,
    duration: f32,
    elapsed: f32,
}
pub struct LensDirtTrack<T> {
    registered_default: T,
    published: LensDirtBlend<T>,
    transition: Option<Transition<T>>,
}
impl<T: Clone> LensDirtTrack<T> {
    pub fn new(registered_default: T) -> Self {
        Self {
            published: LensDirtBlend {
                from: registered_default.clone(),
                to: registered_default.clone(),
                weight: 0.0,
            },
            registered_default,
            transition: None,
        }
    }
    /// `selection` is the last explicit containing-area/world assignment, or
    /// None when no environment supplies the parameter. `parameters_initialized`
    /// is the source parameter map's pre-update nonempty flag, shared by all
    /// tracks; it is not this particular image track's initialization state.
    pub fn update(
        &mut self,
        selection: Option<(EnvironmentSource, T)>,
        duration: f32,
        delta: f32,
        parameters_initialized: bool,
    ) {
        let source = selection.as_ref().map(|v| v.0);
        let selected = selection
            .as_ref()
            .map(|v| v.1.clone())
            .unwrap_or_else(|| self.registered_default.clone());
        if selection.is_some() {
            // Every selected image assignment republishes this baseline before
            // a still-active cached transition may overwrite it.
            self.published = LensDirtBlend {
                from: selected.clone(),
                to: selected.clone(),
                weight: 0.0,
            };
        } else {
            self.published.from = self.registered_default.clone();
            self.published.to = self.registered_default.clone();
        }
        let changed = self.transition.as_ref().is_some_and(|v| v.source != source);
        let new_world = self.transition.is_none() && source == Some(EnvironmentSource::World);
        if changed || (self.transition.is_none() && source.is_some()) {
            self.transition = Some(Transition {
                source,
                target: selected,
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
                self.published = LensDirtBlend {
                    from: self.registered_default.clone(),
                    to: track.target.clone(),
                    weight: (track.elapsed / track.duration.max(MIN_BLEND_DURATION))
                        .clamp(0.0, 1.0),
                };
                retire = track.source.is_none() && track.elapsed >= track.duration;
            }
        }
        if retire {
            self.transition = None;
        }
    }
    pub fn apply_overlay(&mut self, target: T, weight: f32) {
        self.published.apply_overlay(target, weight);
    }
    pub fn value(&self) -> &LensDirtBlend<T> {
        &self.published
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed levels and registered lens-dirt declarations"]
    fn installed_lens_dirt_inputs_resolve() {
        use crate::{
            assets::{installation, level::LevelResources},
            entities, material, texture,
        };
        let catalog = Catalog::open(&installation::game_base().unwrap()).unwrap();
        let mut resident = std::collections::BTreeMap::new();
        let mut levels = 0;
        let mut area_count = 0;
        let mut assignments = 0;
        for path in catalog.paths_with_suffix(".bwm") {
            let level = LevelResources::resolve(&catalog, &path).unwrap();
            let Some(bytes) = level.read_entities(&catalog).unwrap() else {
                continue;
            };
            let document = entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
            let parameters = if let Some(name) = entities::world_environment(&document) {
                let bytes = catalog
                    .read(&format!("generated/decls/env/{name}.decl"))
                    .unwrap()
                    .unwrap();
                entities::environment_render_parameters_with_inheritance(
                    &name,
                    &String::from_utf8_lossy(&bytes),
                    |parent| {
                        catalog
                            .read(&format!("generated/decls/env/{parent}.decl"))
                            .map(|v| v.map(|b| String::from_utf8_lossy(&b).into_owned()))
                            .map_err(|e| e.to_string())
                    },
                )
                .unwrap()
            } else {
                EnvironmentRenderParameters { layers: Vec::new() }
            };
            let areas =
                super::super::environment::load_area_environments(&document, &catalog).unwrap();
            let inputs = PreparedLensDirt::load(&catalog, &parameters, &areas, |name| {
                let path = material::image_load_resource(&catalog, name)
                    .ok_or_else(|| format!("unresolved lens dirt {name}"))?;
                if !resident.contains_key(&path) {
                    if material::intrinsic_image_pixels(&path).is_some() {
                        resident.insert(path.clone(), None);
                    } else {
                        let bytes = catalog
                            .read(&path)
                            .map_err(|e| e.to_string())?
                            .ok_or_else(|| format!("missing lens dirt {path}"))?;
                        let chain = texture::compressed_inline_chain(&bytes)?;
                        resident.insert(path.clone(), Some(chain.info));
                    }
                }
                Ok(path)
            })
            .unwrap();
            let mut track = LensDirtTrack::new(inputs.registered_default.clone());
            inputs
                .update(&mut track, &areas, [1e20; 3], 0.5, 0.1, false, None, &[])
                .unwrap();
            assert_eq!(
                &track.value().to,
                inputs.world.as_ref().unwrap_or(&inputs.registered_default)
            );
            inputs
                .update(&mut track, &areas, [1e20; 3], 0.5, 0.1, true, None, &[])
                .unwrap();
            assert_eq!(track.value().from, track.value().to);
            area_count += areas.len();
            assignments += inputs.areas.iter().filter(|v| v.1.is_some()).count();
            levels += 1;
        }
        assert_eq!(levels, catalog.paths_with_suffix(".bwm").len());
        eprintln!("{levels} levels / {area_count} area snapshots / {assignments} authored area image assignments / {} resident source identities: {resident:?}",resident.len());
    }

    #[test]
    fn image_transitions_match_installed_update_routine() {
        let mut track = LensDirtTrack::new(100u32);
        let mut cases = 0;
        let mut scenarios = 0;
        for line in include_str!("../../tests/fixtures/lens_dirt_track.txt").lines() {
            if line == "RESET" {
                track = LensDirtTrack::new(100);
                scenarios += 1;
                continue;
            }
            let w: Vec<_> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let f = |i| f32::from_bits(w[i]);
            let selection = if w[9] != 0 {
                Some((EnvironmentSource::Secondary(0), w[9]))
            } else {
                match w[1] {
                    2 => Some((EnvironmentSource::Area(1), w[3])),
                    1 => Some((EnvironmentSource::Area(0), w[2])),
                    _ if w[0] != 0 => Some((EnvironmentSource::World, 101)),
                    _ => None,
                }
            };
            let duration = if w[9] != 0 {
                0.0
            } else if w[1] == 2 {
                f(6)
            } else {
                f(5)
            };
            track.update(selection, duration, f(4), w[8] != 0);
            for i in 0..w[7] as usize {
                track.apply_overlay(w[10 + 2 * i], f(11 + 2 * i));
            }
            let value = track.value();
            assert_eq!(
                [value.from, value.to, value.weight.to_bits()],
                w[18..21],
                "case{cases}: {line}"
            );
            cases += 1;
        }
        assert_eq!((cases, scenarios), (74, 17));
    }
}
