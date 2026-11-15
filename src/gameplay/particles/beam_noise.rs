//! Beam noise: 14010e2e0's octave sum over 14010dff0, a two-dimensional
//! simplex noise (skew 0.366..., unskew 0.211..., eight gradients, scale
//! 40) hashed with the Perlin permutation at 141ae8930. That table holds
//! only 256 entries, but the hash indexes up to 512 past its start (and
//! one before it at a zero input), reading the constants stored around it.
//! The window is therefore read from the installed executable.
use crate::float::{add, multiply, subtract};
use std::path::Path;

const TABLE_RVA: u32 = 0x1ae8930;
const BEFORE: usize = 0x200;

pub struct SimplexTable {
    bytes: Vec<u8>,
}

impl SimplexTable {
    pub fn load(game_base: &Path) -> Result<Self, String> {
        let path = game_base
            .parent()
            .ok_or("asset directory has no installation parent")?
            .join("Dishonored2.exe");
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if blake3::hash(&bytes).to_hex().as_str() != crate::assets::class_metadata::IMAGE_HASH {
            return Err("beam noise requires the audited installed executable".into());
        }
        let image = crate::assets::executable::Image::parse(&bytes)?;
        let window = image.read(TABLE_RVA - BEFORE as u32, BEFORE + 0x201)?;
        Ok(Self { bytes: window.to_vec() })
    }

    fn hash(&self, index: i32) -> Result<i32, String> {
        let at = BEFORE as i64 + i64::from(index);
        self.bytes
            .get(usize::try_from(at).map_err(|_| "beam noise hash below its window")?)
            .map(|&value| i32::from(value))
            .ok_or_else(|| format!("beam noise hash index {index} outside its window"))
    }

    /// 14010dff0.
    pub fn simplex(&self, x: f32, y: f32) -> Result<f32, String> {
        let f2 = f32::from_bits(0x3ebb_67ae);
        let g2 = f32::from_bits(0x3e58_658c);
        let g2_twice = f32::from_bits(0x3ed8_658c);
        let s = multiply(add(x, y), f2);
        // The lattice steps down for non-positive inputs, including zero.
        let floor = |value: f32| value as i32 - i32::from(!(value > 0.));
        let (i, j) = (floor(add(s, x)), floor(add(s, y)));
        let t = multiply((i.wrapping_add(j)) as f32, g2);
        let x0 = subtract(x, subtract(i as f32, t));
        let y0 = subtract(y, subtract(j as f32, t));
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = add(subtract(x0, i1 as f32), g2);
        let y1 = add(subtract(y0, j1 as f32), g2);
        let x2 = add(subtract(x0, 1.), g2_twice);
        let y2 = add(subtract(y0, 1.), g2_twice);
        let ii = i % 256;
        let jj = j % 256;
        let corner = |x: f32, y: f32, hash: &dyn Fn() -> Result<i32, String>| -> Result<f32, String> {
            let mut t = subtract(subtract(0.5, multiply(x, x)), multiply(y, y));
            if 0. > t {
                return Ok(0.);
            }
            t = multiply(t, t);
            let h = hash()? & 7;
            let (mut u, mut v) = if h < 4 { (x, y) } else { (y, x) };
            if h & 1 != 0 {
                u = -u;
            }
            v = multiply(v, if h & 2 != 0 { -2. } else { 2. });
            Ok(multiply(add(v, u), multiply(t, t)))
        };
        let n0 = corner(x0, y0, &|| self.hash(ii + self.hash(jj)?))?;
        let n1 = corner(x1, y1, &|| self.hash(ii + i1 + self.hash(jj + j1)?))?;
        let n2 = corner(x2, y2, &|| self.hash(ii + self.hash(jj + 1)? + 1))?;
        Ok(multiply(add(add(n1, n0), n2), 40.))
    }

    /// 14010e2e0: the noise is sampled once; each octave adds it scaled by
    /// persistence^k through (1 - |sum|).
    pub fn octaves(&self, octaves: i32, persistence: f32, frequency: f32, x: f32, t: f32) -> Result<f32, String> {
        let mut sum = 0_f32;
        if 0 < octaves {
            let noise = self.simplex(x, multiply(frequency, t))?;
            let mut amplitude = 1_f32;
            for _ in 0..octaves {
                let weighted = multiply(noise, amplitude);
                amplitude = multiply(amplitude, persistence);
                sum = add(sum, multiply(subtract(1., sum.abs()), weighted));
            }
        }
        Ok(sum)
    }
}
