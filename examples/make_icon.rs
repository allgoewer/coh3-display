//! Writes `installer/coh3-display.ico`, the icon of the exe, its shortcuts and the installer.
//! Run with `cargo run --example make_icon` after changing the drawing in `src/icon.rs`; the
//! result is checked in.

use std::fs;
use std::path::Path;

use coh3_display::icon;

/// One image per size Windows asks for at the usual scalings, so none has to be resampled.
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// One icon image as a 32-bit BMP without file header, which is how ICO stores bitmaps:
/// the height counts the colour rows plus an (empty) 1-bit mask, and rows run bottom-up.
fn image(size: u32) -> Vec<u8> {
    let mask_row = size.div_ceil(32) * 4;
    let mut data = Vec::new();
    data.extend_from_slice(&40u32.to_le_bytes());
    data.extend_from_slice(&(size as i32).to_le_bytes());
    data.extend_from_slice(&(size as i32 * 2).to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&32u16.to_le_bytes());
    data.extend_from_slice(&[0; 24]);

    let rgba = icon::render(size, true);
    for row in rgba.chunks_exact(size as usize * 4).rev() {
        for pixel in row.as_chunks::<4>().0 {
            data.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    data.resize(data.len() + (mask_row * size) as usize, 0);
    data
}

fn main() -> std::io::Result<()> {
    let images: Vec<Vec<u8>> = SIZES.iter().map(|size| image(*size)).collect();

    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(SIZES.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * SIZES.len() as u32;
    for (size, image) in SIZES.iter().zip(&images) {
        // A width or height of 0 means 256.
        let dimension = if *size == 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dimension, dimension, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(image.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += image.len() as u32;
    }
    for image in &images {
        ico.extend_from_slice(image);
    }

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("installer").join("coh3-display.ico");
    fs::create_dir_all(path.parent().expect("has a parent"))?;
    fs::write(&path, ico)?;
    println!("wrote {}", path.display());
    Ok(())
}
