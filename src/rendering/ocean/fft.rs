//! Six radix-eight passes from 1402f9260 and its dispatch wrapper 1402f9790.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FftBuffer {
    Input,
    Scratch,
    Output,
}

#[derive(Clone, Copy, Debug)]
pub struct FftPass {
    pub source: FftBuffer,
    pub destination: FftBuffer,
    pub thread_count: u32,
    pub output_stride: u32,
    pub input_stride: u32,
    pub phase_stride: u32,
    pub phase_base: f32,
    /// Selects owner+18 rather than owner+10 when input stride is below two.
    pub final_radix: bool,
    pub workgroups: [u32; 3],
}

/// Prepare the exact source ping-pong order and constants. This is a dispatch
/// plan: its buffers/programs must come from the real wave-resource owner.
pub fn ocean_fft_passes(size: u32, channels: u32) -> [FftPass; 6] {
    let square = size.wrapping_mul(size);
    let threads = channels.wrapping_mul(square) >> 3;
    let mut phase = f64::from_bits(0xc019_21fb_5444_2d18) / f64::from(square);
    std::array::from_fn(|index| {
        let input_stride = square >> (3 * (index + 1));
        let pass = FftPass {
            source: if index == 0 {
                FftBuffer::Input
            } else if index % 2 == 1 {
                FftBuffer::Scratch
            } else {
                FftBuffer::Output
            },
            destination: if index % 2 == 0 {
                FftBuffer::Scratch
            } else {
                FftBuffer::Output
            },
            thread_count: threads,
            output_stride: square >> if index < 3 { 3 } else { 12 },
            input_stride,
            phase_stride: if index < 3 { 512 } else { 1 },
            phase_base: phase as f32,
            final_radix: input_stride < 2,
            workgroups: [threads >> 7, 1, 1],
        };
        phase *= 8.0;
        pass
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_passes_match_executed_setup_and_dispatch() {
        let resource = |buffer| match buffer {
            FftBuffer::Input => 0x1000,
            FftBuffer::Scratch => 0x2000,
            FftBuffer::Output => 0x3000,
        };
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ocean_fft_passes.txt").lines() {
            let row = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(row.len(), 14);
            let pass = ocean_fft_passes(row[0], row[1])[row[2] as usize];
            let actual = [
                if pass.final_radix { 0xa1 } else { 0xa0 },
                resource(pass.source),
                resource(pass.destination),
                pass.thread_count,
                pass.output_stride,
                pass.input_stride,
                pass.phase_stride,
                pass.phase_base.to_bits(),
                pass.workgroups[0],
                pass.workgroups[1],
                pass.workgroups[2],
            ];
            assert_eq!(actual.as_slice(), &row[3..], "FFT pass {count}");
            count += 1;
        }
        assert_eq!(count, 120);
    }
}
