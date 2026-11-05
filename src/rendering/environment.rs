//! Camera-area render parameter selection (14088d060) and float blending
//! (14088e220). Area containment is the shared trigger shape test.
use crate::{archive, entities, gameplay::trigger_volume::TriggerVolume};
mod volumes;
pub use volumes::{load_environment_volumes, EnvironmentVolume};
pub(crate) use volumes::RuntimeEnvironmentVolume;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentSource {
    World,
    Area(usize),
    Secondary(usize),
}

pub struct AreaEnvironment {
    pub id: String,
    pub parameters: entities::EnvironmentRenderParameters,
    pub blend_duration: f32,
    volume: TriggerVolume,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters(value: f32) -> entities::EnvironmentRenderParameters {
        entities::EnvironmentRenderParameters {
            layers: vec![entities::EnvironmentRenderParameterLayer {
                resource: "test".into(),
                parameters: vec![("test/value".into(), value.to_string())],
            }],
        }
    }

    #[test]
    fn area_float_track_restarts_from_current_value_when_leaving_during_a_blend() {
        let world = parameters(70.0);
        let area = AreaEnvironment {
            id: "test".into(),
            parameters: parameters(90.0),
            blend_duration: 1.0,
            volume: TriggerVolume {
                bounds: [[-1.0; 3], [1.0; 3]],
                hulls: vec![vec![
                    [1.0, 0.0, 0.0, -1.0],
                    [-1.0, 0.0, 0.0, -1.0],
                    [0.0, 1.0, 0.0, -1.0],
                    [0.0, -1.0, 0.0, -1.0],
                    [0.0, 0.0, 1.0, -1.0],
                    [0.0, 0.0, -1.0, -1.0],
                ]],
                hull_vertices: vec![Vec::new()],
            },
        };
        let mut track = EnvironmentScalar::new(70.0);
        let areas = [area];
        // Establish the initial world before entering an area: the first
        // selection itself snaps, while subsequent changes interpolate.
        assert_eq!(
            track
                .update(&world, 1.0, &areas, [2.0; 3], "test/value", 0.1)
                .unwrap(),
            70.0
        );
        assert_eq!(
            track
                .update(&world, 1.0, &areas, [0.0; 3], "test/value", 0.25)
                .unwrap(),
            75.0
        );
        assert_eq!(
            track
                .update(&world, 1.0, &areas, [0.0; 3], "test/value", 0.25)
                .unwrap(),
            80.0
        );
        assert_eq!(
            track
                .update(&world, 1.0, &areas, [2.0; 3], "test/value", 0.5)
                .unwrap(),
            75.0
        );
        assert_eq!(
            track
                .update(&world, 1.0, &areas, [2.0; 3], "test/value", 0.5)
                .unwrap(),
            70.0
        );
    }

    #[test]
    #[ignore = "requires installed Dishonored 2; compares the live captured Dust area query"]
    fn installed_area_query_matches_loaded_dust_floor_and_both_camera_captures() {
        let catalog =
            archive::Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        for map in ["dust_p_lowchaos", "eotw_p_lowchaos"] {
            let level = crate::assets::level::LevelResources::resolve(&catalog, map).unwrap();
            let bytes = level.read_entities(&catalog).unwrap().unwrap();
            let document = entities::parse(&String::from_utf8_lossy(&bytes)).unwrap();
            let areas = load_area_environments(&document, &catalog).unwrap();
            assert!(!areas.is_empty());
            if map.starts_with("dust") {
                for point in [
                    [12.709995, -10.707173, -19.346228],
                    [12.596064, -10.935466, -19.385145],
                    [13.371951, -10.32991, -19.246834],
                ] {
                    let selected: Vec<_> = areas.iter().filter(|a| a.contains(point)).collect();
                    assert_eq!(
                        selected.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
                        ["audio_area_volume_steepst01"]
                    );
                    assert_eq!(
                        selected[0]
                            .parameters
                            .scalar_parameter("env/shadow/parallelLightExtend")
                            .unwrap(),
                        Some(85.0)
                    );
                }
            }
        }
    }
}

impl AreaEnvironment {
    pub fn contains(&self, point: [f32; 3]) -> bool {
        self.volume.contains(point)
    }
}

pub fn load_area_environments(
    document: &entities::Document,
    catalog: &archive::Catalog,
) -> Result<Vec<AreaEnvironment>, String> {
    load_area_environments_with_sources(document, catalog, &mut Default::default())
}

pub(crate) fn load_area_environments_with_sources(
    document: &entities::Document, catalog: &archive::Catalog,
    sources: &mut crate::physworld::source::ShapeSources,
) -> Result<Vec<AreaEnvironment>, String> {
    let mut result = Vec::new();
    for volume in entities::placed_environment_volumes(document) {
        let resolved = volumes::resolve(volume, catalog)?;
        if let Some(area) = area_from_volume(resolved.placement, resolved.parameters, resolved.is_water_volume, catalog, sources)? { result.push(area); }
    }
    // 88d060 inserts larger physics AABB diagonals first. Later parameter
    // assignments win in 88e220, so smaller containing volumes take priority.
    result.sort_by(|a, b| b.volume.size_squared().total_cmp(&a.volume.size_squared()));
    report_area_signatures(&result);
    Ok(result)
}

/// Publish only the values needed by the physics and water owners, moving the
/// declaration parameters into the area owner when both use this acquisition.
pub(crate) fn prepare_volume_owners(
    volumes: Vec<EnvironmentVolume>, catalog: &archive::Catalog,
    separate_areas: Option<Vec<AreaEnvironment>>, sources: &mut crate::physworld::source::ShapeSources,
) -> Result<(Vec<AreaEnvironment>, Vec<RuntimeEnvironmentVolume>), String> {
    let make_areas = separate_areas.is_none();
    let mut areas = separate_areas.unwrap_or_default();
    let retain = std::env::var("DH2_DEBUG_RETAIN_ENVIRONMENT_PROFILES").as_deref() == Ok("1");
    let trace = std::env::var("DH2_DEBUG_ENVIRONMENT_ACQUISITIONS").as_deref() == Ok("1");
    let mut parameter_capacity = 0usize;
    let mut placement_capacity = 0usize;
    let input_vector_capacity = volumes.capacity() * std::mem::size_of::<EnvironmentVolume>();
    let mut live = Vec::with_capacity(volumes.len());
    for volume in volumes {
        if trace {
            parameter_capacity += profile_capacity(&volume.parameters);
            placement_capacity += volume.placement.environment_decl.capacity()
                + volume.placement.model.as_ref().map_or(0, String::capacity)
                + volume.placement.scattering_source.as_ref().map_or(0, String::capacity);
        }
        if retain {
            if make_areas && !volume.is_water_volume {
                if let Some(area) = area_from_volume(volume.placement.clone(), volume.parameters.clone(), false, catalog, sources)? { areas.push(area); }
            }
            live.push(RuntimeEnvironmentVolume { id: volume.placement.id.clone(), position: volume.placement.position,
                is_water_volume: volume.is_water_volume, surface_material: volume.surface_material.clone(), _retained: Some(Box::new(volume)) });
        } else {
            let EnvironmentVolume { placement, parameters, is_water_volume, surface_material } = volume;
            live.push(RuntimeEnvironmentVolume { id: placement.id.clone(), position: placement.position,
                is_water_volume, surface_material, _retained: None });
            if make_areas {
                if let Some(area) = area_from_volume(placement, parameters, is_water_volume, catalog, sources)? { areas.push(area); }
            }
        }
    }
    if make_areas {
        areas.sort_by(|a, b| b.volume.size_squared().total_cmp(&a.volume.size_squared()));
        report_area_signatures(&areas);
    }
    if trace {
        let mut crc = flate2::Crc::new();
        for volume in &live { crc.update(format!("{}{:?}{:?}{:?}", volume.id, volume.position, volume.is_water_volume, volume.surface_material).as_bytes()); crc.update(&[0]); }
        eprintln!("MEMORY environment volume retirement: retained={retain} volumes={} parameter_capacity={parameter_capacity} placement_capacity={placement_capacity} input_vector_capacity={input_vector_capacity} runtime_vector_capacity={} signature={:08x}", live.len(), live.capacity() * std::mem::size_of::<RuntimeEnvironmentVolume>(), crc.sum());
    }
    Ok((areas, live))
}

fn report_area_signatures(areas: &[AreaEnvironment]) {
    if std::env::var("DH2_DEBUG_ENVIRONMENT_ACQUISITIONS").as_deref() != Ok("1") { return; }
    let mut crc = flate2::Crc::new();
    let mut parameter_capacity = 0usize;
    for area in areas {
        crc.update(area.id.as_bytes());
        crc.update(format!("{:?}{:?}{:?}", area.parameters, area.blend_duration, area.volume).as_bytes());
        crc.update(&[0]);
        parameter_capacity += profile_capacity(&area.parameters);
    }
    eprintln!("MEMORY area environment output: count={} signature={:08x} parameter_capacity={parameter_capacity}", areas.len(), crc.sum());
}

fn profile_capacity(parameters: &entities::EnvironmentRenderParameters) -> usize {
    parameters.layers.capacity() * std::mem::size_of::<entities::EnvironmentRenderParameterLayer>()
        + parameters.layers.iter().map(|layer| layer.resource.capacity()
            + layer.parameters.capacity() * std::mem::size_of::<(String, String)>()
            + layer.parameters.iter().map(|(key, value)| key.capacity() + value.capacity()).sum::<usize>()).sum::<usize>()
}

fn area_from_volume(volume: entities::PlacedEnvironmentVolume, parameters: entities::EnvironmentRenderParameters,
    is_water_volume: bool, catalog: &archive::Catalog, sources: &mut crate::physworld::source::ShapeSources) -> Result<Option<AreaEnvironment>, String> {
    // 88d060 keeps water volumes in a separate query result.
    if is_water_volume { return Ok(None); }
    let model = volume.model.as_deref().ok_or_else(|| format!("{}: area trigger has no model", volume.id))?;
    let shape = TriggerVolume::load_with_sources(catalog, &volume.id, model, volume.position, volume.rotation,
        sources, crate::physworld::source::ShapeUse::Area)?;
    Ok(Some(AreaEnvironment {
        id: volume.id, parameters,
        blend_duration: entities::environment_blend_duration(&volume.environment_decl)?,
        volume: shape,
    }))
}

/// A typed float parameter track from 88e220: switching source restarts the
/// blend from the currently displayed value, even when targets happen to match.
pub type EnvironmentScalar = EnvironmentTrack<f32>;
pub type EnvironmentVector = EnvironmentTrack<bevy_math::Vec4>;

pub(crate) const MIN_BLEND_DURATION: f32 = f32::from_bits(0x38d1_b717);

/// The original float track blends all four words component by component.
/// Scalar and float4 consumers share the same source-change and timing rules.
pub struct EnvironmentTrack<T> {
    source: Option<usize>,
    initialized: bool,
    start: T,
    target: T,
    current: T,
    elapsed: f32,
    duration: f32,
}

impl<T: Clone> EnvironmentTrack<T> {
    pub fn new(registered_default: T) -> Self {
        Self {
            source: None,
            initialized: false,
            start: registered_default.clone(),
            target: registered_default.clone(),
            current: registered_default,
            elapsed: 0.0,
            duration: 0.0,
        }
    }
}

fn area_parameter<T>(
    world: &entities::EnvironmentRenderParameters,
    world_duration: f32,
    areas: &[AreaEnvironment],
    point: [f32; 3],
    name: &str,
    read: impl Fn(&entities::EnvironmentRenderParameters) -> Result<Option<T>, String>,
) -> Result<(Option<usize>, T, f32), String> {
    let mut source = None;
    let mut target = read(world)?.ok_or_else(|| format!("unresolved environment {name}"))?;
    let mut duration = world_duration;
    for (index, area) in areas.iter().enumerate().filter(|(_, a)| a.contains(point)) {
        if let Some(value) = read(&area.parameters)? {
            source = Some(index);
            target = value;
            duration = area.blend_duration;
        }
    }
    Ok((source, target, duration))
}

impl EnvironmentTrack<f32> {
    pub fn update(
        &mut self,
        world: &entities::EnvironmentRenderParameters,
        world_duration: f32,
        areas: &[AreaEnvironment],
        point: [f32; 3],
        name: &str,
        delta: f32,
    ) -> Result<f32, String> {
        let (source, target, duration) =
            area_parameter(world, world_duration, areas, point, name, |p| {
                p.scalar_parameter(name)
            })?;
        Ok(self.advance(source, target, duration, delta))
    }
}

impl EnvironmentTrack<bevy_math::Vec4> {
    pub fn update(
        &mut self,
        world: &entities::EnvironmentRenderParameters,
        world_duration: f32,
        areas: &[AreaEnvironment],
        point: [f32; 3],
        name: &str,
        delta: f32,
    ) -> Result<bevy_math::Vec4, String> {
        let (source, target, duration) =
            area_parameter(world, world_duration, areas, point, name, |p| {
                p.vector_parameter(name)
                    .map(|value| value.map(bevy_math::Vec4::from_array))
            })?;
        Ok(self.advance(source, target, duration, delta))
    }
}

impl<T> EnvironmentTrack<T>
where
    T: Copy
        + std::ops::Sub<Output = T>
        + std::ops::Add<Output = T>
        + std::ops::Mul<f32, Output = T>,
{
    fn advance(&mut self, source: Option<usize>, target: T, duration: f32, delta: f32) -> T {
        self.advance_with(
            source,
            target,
            duration,
            delta,
            |start, target, fraction| (*target - *start) * fraction + *start,
        )
    }
}

impl<T: Clone> EnvironmentTrack<T> {
    pub(crate) fn advance_with(
        &mut self,
        source: Option<usize>,
        target: T,
        duration: f32,
        delta: f32,
        blend: impl Fn(&T, &T, f32) -> T,
    ) -> T {
        if !self.initialized || source != self.source {
            self.start = self.current.clone();
            self.target = target;
            // 88e220 initializes the first selection immediately, including
            // an area. Later source changes use the authored blend duration.
            self.duration = if !self.initialized { 0.0 } else { duration };
            self.elapsed = 0.0;
            self.source = source;
            self.initialized = true;
        }
        if self.elapsed <= self.duration {
            self.elapsed += delta;
            let fraction = (self.elapsed / self.duration.max(MIN_BLEND_DURATION)).clamp(0.0, 1.0);
            self.current = blend(&self.start, &self.target, fraction);
        }
        self.current.clone()
    }
}

#[cfg(test)]
mod initialization_tests {
    use super::*;

    #[test]
    fn static_world_and_area_initialization_matches_executed_updates() {
        let fixture = include_str!("../../tests/fixtures/environment_float_track.txt");
        let mut frames = 0;
        for scenario in fixture
            .split("RESET")
            .filter(|text| !text.trim().is_empty())
        {
            let rows: Vec<Vec<u32>> = scenario
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    line.split_whitespace()
                        .map(|word| u32::from_str_radix(word, 16).unwrap())
                        .collect()
                })
                .collect();
            // This API represents a static world and ordinary areas. Presence,
            // secondary sources and overlays have their own ParameterTrack.
            if rows
                .iter()
                .any(|row| row[0] == 0 || row[7] != 0 || row[9] != 0)
            {
                continue;
            }
            let mut track = EnvironmentTrack::new(-0.0_f32);
            for row in rows {
                let (source, target, duration) = match row[1] {
                    0 => (None, 5.0, 0.6), // owned executable oracle's world inputs
                    1 => (Some(0), row[2] as f32, f32::from_bits(row[5])),
                    2 => (Some(1), row[3] as f32, f32::from_bits(row[6])),
                    count => panic!("unexpected area count {count}"),
                };
                let actual = track.advance(source, target, duration, f32::from_bits(row[4]));
                assert_eq!(
                    actual.to_bits(),
                    row[row.len() - 3],
                    "executed frame{frames}"
                );
                frames += 1;
            }
        }
        assert_eq!(frames, 36);
    }
}
