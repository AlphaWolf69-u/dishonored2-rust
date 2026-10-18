//! Reader for the installed Dishonored 2 version-14 environment-probe payload.
//!
//! Field order and legacy branches follow the saved FUN_1402b99a0,
//! FUN_1402b8b30, FUN_1402b9720, FUN_1402b8950, FUN_1402b98b0 and
//! FUN_1402b8a50 exports. The reader keeps the authored SH coefficients and
//! sample records; it does not choose or blend probes.

use std::convert::TryInto;

#[derive(Clone, Debug)]
pub struct Sample {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub values: [f32; 5],
    pub flags: [u16; 2],
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub marker: u8,
    pub version: u32,
    pub generation: u32,
    /// Authored probe world position serialized at arkEnvProbe +0x30.
    pub position: [f32; 3],
    /// results_t illumination metadata at offsets +0x30..+0x4c.
    pub static_lights_illuminance: [f32; 3],
    pub sky_occlusion_factor: f32,
    pub sun_irradiance_factor: f32,
    pub illuminance_estimate: [f32; 3],
    pub sources:ProbeSources,
    pub static_lighting_sh: [[f32; 3]; 9],
    pub sun_factor_sh: [[f32; 3]; 9],
    pub filtered_sun_factor_sh: [[f32; 3]; 9],
    pub sky_occlusion_sh: [f32; 9],
    pub filtered_sky_occlusion_sh: [f32; 9],
    pub samples: Vec<Sample>,
    pub cubemap_name: String,
    pub has_additional_results: bool,
    pub bytes_consumed: usize,
}

#[derive(Clone,Debug)]
pub struct ProbeSources {
    /// Header vectors at +44/+50/+5c/+68: sun, sky, dynamic, static.
    pub bounce_factors: [[f32;3];4],
    pub sky_scale:f32,
    pub lighting_scale:f32,
    pub ambient_factor:[f32;3],
    pub multi_bounce_static_sh:[[f32;3];9],
    pub multi_bounce_sun_sh:[[f32;3];9],
    pub multi_bounce_sky_sh:[[f32;3];9],
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], cursor: usize) -> Self {
        Self { bytes, cursor }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(count)
            .ok_or_else(|| "probe offset overflow".to_owned())?;
        let value = self.bytes.get(self.cursor..end).ok_or_else(|| {
            format!(
                "probe truncated at {:#x} reading {count:#x} bytes",
                self.cursor
            )
        })?;
        self.cursor = end;
        Ok(value)
    }

    fn skip(&mut self, count: usize) -> Result<(), String> {
        self.take(count).map(|_| ())
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, String> {
        let length = self.u32()? as usize;
        if length > 1 << 20 {
            return Err(format!(
                "probe string length is unreasonable: {length} at {:#x}",
                self.cursor - 4
            ));
        }
        let bytes = self.take(length)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn count(&mut self) -> Result<usize, String> {
        let count = self.u32()? as i32;
        if !(0..=1_000_000).contains(&count) {
            return Err(format!("probe list count is invalid: {count}"));
        }
        Ok(count as usize)
    }

    fn counted_strings(&mut self, count: usize, fixed_bytes: usize) -> Result<(), String> {
        for _ in 0..count {
            self.string()?;
            self.skip(fixed_bytes)?;
        }
        Ok(())
    }
}

fn read_sh3(reader: &mut Reader<'_>) -> Result<[[f32; 3]; 9], String> {
    let mut coefficients = [[0.0; 3]; 9];
    for coefficient in &mut coefficients {
        for channel in coefficient {
            *channel = reader.f32()?;
        }
    }
    Ok(coefficients)
}

fn read_sh1(reader: &mut Reader<'_>) -> Result<[f32; 9], String> {
    let mut coefficients = [0.0; 9];
    for coefficient in &mut coefficients {
        *coefficient = reader.f32()?;
    }
    Ok(coefficients)
}

/// Decode the version-14 PRB format used by all installed game probes.
pub fn parse(bytes: &[u8]) -> Result<Probe, String> {
    if bytes.len() < 12 || bytes.get(..3) != Some(b"PRB") {
        return Err("invalid or truncated PRB header".into());
    }
    let marker = bytes[3];
    if marker < 11 {
        return Err(format!("unsupported legacy PRB marker {marker}"));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let generation = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if version != 14 {
        return Err(format!("unsupported probe loader version {version}"));
    }

    let mut reader = Reader::new(bytes, 12);

    // FUN_1402b99a0's environment-probe header fields before results_t.
    let position = [reader.f32()?, reader.f32()?, reader.f32()?];
    reader.skip(4+1)?;
    let mut bounce_factors=[[0.0;3];4];
    for factor in &mut bounce_factors {for v in factor {*v=reader.f32()?;}}
    let sky_scale=reader.f32()?;let lighting_scale=reader.f32()?;
    let ambient_factor=[reader.f32()?,reader.f32()?,reader.f32()?];
    reader.skip(2+4)?;

    // FUN_1402b8b30 reads results_t metadata preceding its SH arrays and
    // m_samples at +0x750. Reflection metadata names the 32 bytes at +0x30:
    // static-light illuminance, sky-occlusion factor, sun-irradiance factor,
    // and illuminance estimate.
    reader.skip(4 * 4 + 12 + 12)?;
    let static_lights_illuminance = [reader.f32()?, reader.f32()?, reader.f32()?];
    let sky_occlusion_factor = reader.f32()?;
    let sun_irradiance_factor = reader.f32()?;
    let illuminance_estimate = [reader.f32()?, reader.f32()?, reader.f32()?];
    reader.skip(9 * 4)?;

    let static_lighting_sh = read_sh3(&mut reader)?;
    let sun_factor_sh = read_sh3(&mut reader)?;
    let filtered_sun_factor_sh = read_sh3(&mut reader)?;
    let sky_occlusion_sh = read_sh1(&mut reader)?;
    let filtered_sky_occlusion_sh = read_sh1(&mut reader)?;
    reader.skip(0x400)?; // Environment-distance SH table.
    let multi_bounce_static_sh=read_sh3(&mut reader)?;
    let multi_bounce_sun_sh=read_sh3(&mut reader)?;
    let multi_bounce_sky_sh=read_sh3(&mut reader)?;
    reader.skip(12)?; // Distance sample min/max/mean.

    let sample_count = reader.u32()?;
    if sample_count != 128 {
        return Err(format!(
            "saved version-14 reader expects 128 samples, got {sample_count}"
        ));
    }
    let mut samples = Vec::with_capacity(sample_count as usize);
    for _ in 0..sample_count {
        let mut position = [0.0; 3];
        let mut direction = [0.0; 3];
        for value in &mut position {
            *value = reader.f32()?;
        }
        for value in &mut direction {
            *value = reader.f32()?;
        }
        let mut values = [0.0; 5];
        for value in &mut values {
            *value = reader.f32()?;
        }
        let flags = [
            u16::from_le_bytes(reader.take(2)?.try_into().unwrap()),
            u16::from_le_bytes(reader.take(2)?.try_into().unwrap()),
        ];
        samples.push(Sample {
            position,
            direction,
            values,
            flags,
        });
    }

    // The saved reader stores these variable-size lists after the 128 samples.
    let emissive_count = reader.count()?;
    reader.counted_strings(emissive_count, 0x28)?;
    reader.skip(8)?; // Probe-list field and farthest-neighbor distance.

    let neighbor_count = reader.count()?;
    for _ in 0..neighbor_count {
        reader.string()?;
        reader.skip(1 + 4 + 4 + 4 + 12 + 1)?;
    }

    let voronoi_count = reader.count()?;
    reader.counted_strings(voronoi_count, 4 + 12 + 12 + 4 + 0x24 + 2)?;

    let occluder_count = reader.count()?;
    reader.counted_strings(occluder_count, 0x6c * 3 + 0x24 * 2 + 4 + 8 + 4)?;

    let cubemap_name = reader.string()?;
    reader.skip(0x60)?; // Serialized virtual-plane records.
                        // FUN_1402b99a0 reads this flag after the primary results_t. A set flag
                        // means two additional results_t payloads follow (including IBL data).
    let has_additional_results = reader.take(1)?[0] != 0;
    if has_additional_results {
        return Err("probe has the optional second/third results payloads; this reader has not ported them yet".into());
    }
    if reader.cursor != bytes.len() {
        return Err(format!(
            "probe reader consumed {:#x} of {:#x} bytes",
            reader.cursor,
            bytes.len()
        ));
    }

    Ok(Probe {
        marker,
        version,
        generation,
        position,
        static_lights_illuminance,
        sky_occlusion_factor,
        sun_irradiance_factor,
        illuminance_estimate,
        sources:ProbeSources {bounce_factors,sky_scale,lighting_scale,ambient_factor,
        multi_bounce_static_sh,multi_bounce_sun_sh,multi_bounce_sky_sh},
        static_lighting_sh,
        sun_factor_sh,
        filtered_sun_factor_sh,
        sky_occlusion_sh,
        filtered_sky_occlusion_sh,
        samples,
        cubemap_name,
        has_additional_results,
        bytes_consumed: reader.cursor,
    })
}
