//! Initial H0/Omega data from 1402fa690; no GPU resources or global CRT state.
use crate::float::pack_half_truncated as spectrum_half;

#[derive(Clone, Copy, Debug)]
pub struct WaveSpectrumParameters {
    pub wind: [f32; 2],
    pub amplitude: f32,
    pub wind_speed: f32,
    pub opposite_wind_attenuation: f32,
    pub domain_length: f32,
    /// Owner+1c divides speed²/gravity to obtain the short-wave damping length.
    pub short_wave_divisor: f32,
}

fn random(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    (*state >> 16) & 0x7fff
}

fn gaussian(state: &mut u32) -> f32 {
    let u = (random(state) as f32 / 32767.0).max(f32::from_bits(0x3586_37bd));
    let v = random(state) as f32 / 32767.0;
    let radius = (u.ln() * -2.0).sqrt();
    let mut angle = f32::from_bits(0x40c9_0fdb) * v;
    if angle < 0.0 || angle >= f32::from_bits(0x40c9_0fdb) {
        angle -= (angle * f32::from_bits(0x3e22_f983)).floor() * f32::from_bits(0x40c9_0fdb);
    }
    angle.cos() * radius
}

/// Fill the (size+1)² active cells with row stride size+4, leaving the other
/// three columns untouched, just as the source allocator-backed upload does.
/// Returns the final CRT-compatible random state after the source's seed-zero run.
pub fn initialize_wave_spectrum(
    size: u32,
    parameters: WaveSpectrumParameters,
    h0: &mut [[u16; 2]],
    omega: &mut [u16],
) -> Result<u32, String> {
    let stride = (size as usize)
        .checked_add(4)
        .ok_or("wave spectrum stride overflow")?;
    let rows = (size as usize)
        .checked_add(1)
        .ok_or("wave spectrum row count overflow")?;
    let required = stride
        .checked_mul(rows)
        .ok_or("wave spectrum allocation overflow")?;
    if h0.len() < required || omega.len() < required {
        return Err("wave spectrum buffers are smaller than the padded grid".into());
    }
    let gravity = f32::from_bits(0x411c_f5c3);
    let amplitude = parameters.amplitude * f32::from_bits(0x3a83_126f);
    let mut state = 0;
    for y in 0..=size {
        let ky = (y as f32 - size as f32 * 0.5)
            * (f32::from_bits(0x40c9_0fdb) / parameters.domain_length);
        let ky_squared = ky * ky;
        for x in 0..=size {
            let kx = (x as f32 - size as f32 * 0.5)
                * (f32::from_bits(0x40c9_0fdb) / parameters.domain_length);
            let square = kx * kx + ky_squared;
            let strength = if kx == 0.0 && ky == 0.0 {
                0.0
            } else {
                let length = parameters.wind_speed * parameters.wind_speed / gravity;
                let damping = length / parameters.short_wave_divisor;
                let dot = parameters.wind[0] * kx + parameters.wind[1] * ky;
                let mut power = (-1.0 / (length * length * square)).exp() * amplitude;
                power /= square * square * square;
                power *= dot * dot;
                if dot < 0.0 {
                    power *= parameters.opposite_wind_attenuation;
                }
                ((-(square * damping * damping)).exp() * power).sqrt()
            };
            let index = y as usize * stride + x as usize;
            let scale = f32::from_bits(0x3f35_04f3);
            h0[index] = [
                spectrum_half(gaussian(&mut state) * strength * scale),
                spectrum_half(gaussian(&mut state) * strength * scale),
            ];
            omega[index] = spectrum_half((square.sqrt() * gravity).sqrt());
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_size_wave_spectrum_matches_executed_buffers() {
        fn digest(bytes: impl Iterator<Item = u8>) -> u64 {
            bytes.fold(0xcbf2_9ce4_8422_2325, |value, byte| {
                (value ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
            })
        }
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/ocean_spectrum_hashes.txt").lines() {
            let row = line
                .split_whitespace()
                .map(|v| u64::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let size = row[0] as u32;
            let values: [f32; 7] = std::array::from_fn(|i| f32::from_bits(row[i + 2] as u32));
            let count = (size as usize + 4) * (size as usize + 1);
            let mut h0 = vec![[0xcdcd; 2]; count];
            let mut omega = vec![0xcdcd; count];
            let state = initialize_wave_spectrum(
                size,
                WaveSpectrumParameters {
                    wind: [values[0], values[1]],
                    amplitude: values[2],
                    wind_speed: values[3],
                    opposite_wind_attenuation: values[4],
                    domain_length: values[5],
                    short_wave_divisor: values[6],
                },
                &mut h0,
                &mut omega,
            )
            .unwrap();
            assert_eq!(u64::from(state), row[1]);
            assert_eq!(
                digest(h0.into_iter().flatten().flat_map(u16::to_le_bytes)),
                row[9],
                "case {cases} size {size} H0"
            );
            assert_eq!(
                digest(omega.into_iter().flat_map(u16::to_le_bytes)),
                row[10],
                "case {cases} size {size} Omega"
            );
            cases += 1;
        }
        assert_eq!(cases, 9);
    }

    #[test]
    fn initial_wave_spectrum_matches_executed_initializer() {
        fn take<'a>(data: &mut &'a [u8], count: usize) -> &'a [u8] {
            let (result, tail) = data.split_at(count);
            *data = tail;
            result
        }
        fn word(data: &mut &[u8]) -> u32 {
            u32::from_le_bytes(take(data, 4).try_into().unwrap())
        }
        let mut data = include_bytes!("../../../tests/fixtures/ocean_spectrum.bin").as_slice();
        let mut cases = 0;
        while !data.is_empty() {
            let size = word(&mut data);
            let expected_state = word(&mut data);
            let values: [f32; 7] = std::array::from_fn(|_| f32::from_bits(word(&mut data)));
            let count = (size as usize + 4) * (size as usize + 1);
            let mut h0 = vec![[0xcdcd; 2]; count];
            let mut omega = vec![0xcdcd; count];
            let state = initialize_wave_spectrum(
                size,
                WaveSpectrumParameters {
                    wind: [values[0], values[1]],
                    amplitude: values[2],
                    wind_speed: values[3],
                    opposite_wind_attenuation: values[4],
                    domain_length: values[5],
                    short_wave_divisor: values[6],
                },
                &mut h0,
                &mut omega,
            )
            .unwrap();
            assert_eq!(state, expected_state, "case {cases} random state");
            let expected_h0 = take(&mut data, count * 4)
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .collect::<Vec<_>>();
            let expected_omega = take(&mut data, count * 2)
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                .collect::<Vec<_>>();
            assert_eq!(
                h0.into_iter().flatten().collect::<Vec<_>>(),
                expected_h0,
                "case {cases} H0"
            );
            assert_eq!(omega, expected_omega, "case {cases} Omega");
            cases += 1;
        }
        assert_eq!(cases, 12);
    }
}
