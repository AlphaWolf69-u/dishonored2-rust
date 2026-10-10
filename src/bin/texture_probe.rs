//! Compare block compression modes for an extracted Void `_mip0` resource.
#[cfg(test)]
#[path = "../archive.rs"]
mod archive;
#[path = "../texture.rs"]
mod texture;
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn le_u32(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or("short mip header")?
            .try_into()?,
    ))
}

fn decode(bytes: &[u8], width: usize, height: usize, bc7: bool) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || width % 4 != 0 || height % 4 != 0 {
        return Err("unsupported mip dimensions".into());
    }
    let expected = width.checked_mul(height).ok_or("image size overflow")?;
    if bytes.len() != expected {
        return Err(format!("expected {expected} compressed bytes, got {}", bytes.len()).into());
    }
    let mut output = vec![0; expected * 4];
    for block_y in 0..height / 4 {
        for block_x in 0..width / 4 {
            let block_number = block_y * (width / 4) + block_x;
            let compressed = &bytes[block_number * 16..block_number * 16 + 16];
            let mut pixels = [0u8; 64];
            if bc7 {
                bcdec_rs::bc7(compressed, &mut pixels, 16);
            } else {
                bcdec_rs::bc3(compressed, &mut pixels, 16);
            }
            for row in 0..4 {
                let target = ((block_y * 4 + row) * width + block_x * 4) * 4;
                output[target..target + 16].copy_from_slice(&pixels[row * 16..row * 16 + 16]);
            }
        }
    }
    Ok(output)
}

fn save(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let file = BufWriter::new(File::create_new(path)?);
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn run() -> Result<()> {
    let mut args = env::args().skip(1);
    let input = args
        .next()
        .ok_or("Usage: texture-probe <extracted .bimage7_mip0> <output-prefix>")?;
    let prefix = args.next().ok_or("missing output prefix")?;
    let bytes = fs::read(input)?;
    if bytes.starts_with(b"BIM") {
        for (label, decoder) in [
            (
                "bc7",
                texture::decode_inline_bc7
                    as fn(&[u8]) -> std::result::Result<texture::RgbaImage, String>,
            ),
            (
                "bc3",
                texture::decode_inline_bc3
                    as fn(&[u8]) -> std::result::Result<texture::RgbaImage, String>,
            ),
        ] {
            let decoded = decoder(&bytes)?;
            let output = format!("{prefix}-{label}.png");
            save(
                Path::new(&output),
                decoded.width,
                decoded.height,
                &decoded.pixels,
            )?;
            println!(
                "Wrote {output} ({}x{} inline BIM)",
                decoded.width, decoded.height
            );
        }
        return Ok(());
    }
    let width = le_u32(&bytes, 8)?;
    let height = le_u32(&bytes, 12)?;
    let size = le_u32(&bytes, 16)? as usize;
    if bytes.len() != size + 20 {
        return Err("mip payload size mismatch".into());
    }
    println!("Mip {width}x{height}, {size} compressed bytes");
    for (label, bc7) in [("bc3", false), ("bc7", true)] {
        let output = format!("{prefix}-{label}.png");
        let image = decode(&bytes[20..], width as usize, height as usize, bc7)?;
        save(Path::new(&output), width, height, &image)?;
        println!("Wrote {output}");
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Texture probe error: {error}");
        std::process::exit(1);
    }
}
