//! Mentis' symbol: a round badge with a white microphone. Pure pixel code, shared by the tray
//! (status colors at runtime) and build.rs (program icon file).

/// Badge color of the program icon and of the "ready" tray state.
pub const BLUE: [u8; 3] = [40, 104, 208];

/// Renders the badge as `size`×`size` pixels, 0xAARRGGBB, top row first.
pub fn render(size: usize, base: [u8; 3]) -> Vec<u32> {
    const SAMPLES: usize = 4; // 4×4 supersampling for smooth edges
    let mut pixels = vec![0u32; size * size];
    for (i, pixel) in pixels.iter_mut().enumerate() {
        let (px, py) = (i % size, i / size);
        let (mut disc, mut glyph) = (0u32, 0u32);
        for s in 0..SAMPLES * SAMPLES {
            let x = (px as f32 + ((s % SAMPLES) as f32 + 0.5) / SAMPLES as f32) / size as f32;
            let y = (py as f32 + ((s / SAMPLES) as f32 + 0.5) / SAMPLES as f32) / size as f32;
            if (x - 0.5).powi(2) + (y - 0.5).powi(2) <= 0.25 {
                disc += 1;
                glyph += u32::from(microphone(x, y));
            }
        }
        if disc == 0 {
            continue;
        }
        let alpha = (disc as f32 / (SAMPLES * SAMPLES) as f32 * 255.0).round() as u32;
        let white = glyph as f32 / disc as f32;
        let mix = |c: u8| (f32::from(c) + (255.0 - f32::from(c)) * white).round() as u32;
        *pixel = alpha << 24 | mix(base[0]) << 16 | mix(base[1]) << 8 | mix(base[2]);
    }
    pixels
}

/// Microphone glyph in unit coordinates (0..1, y down).
fn microphone(x: f32, y: f32) -> bool {
    let dx = x - 0.5;
    let head_y = y.clamp(0.30, 0.44);
    let head = dx * dx + (y - head_y).powi(2) <= 0.12 * 0.12;
    let ring = (dx * dx + (y - 0.44).powi(2)).sqrt();
    let holder = y >= 0.44 && (ring - 0.215).abs() <= 0.035;
    let stem = dx.abs() <= 0.035 && (0.65..=0.78).contains(&y);
    let foot = dx.abs() <= 0.14 && (0.75..=0.81).contains(&y);
    head || holder || stem || foot
}

/// Windows .ico file with one 32-bit image per size (BMP entries, alpha channel).
#[allow(dead_code)] // used by build.rs only
pub fn ico(sizes: &[usize], base: [u8; 3]) -> Vec<u8> {
    let mut images = Vec::new();
    for &size in sizes {
        let mask_row = size.div_ceil(32) * 4;
        let mut image = Vec::new();
        // BITMAPINFOHEADER; the height counts the color and the mask bitmap.
        for value in [40u32, size as u32, 2 * size as u32] {
            image.extend(value.to_le_bytes());
        }
        image.extend(1u16.to_le_bytes());
        image.extend(32u16.to_le_bytes());
        image.extend([0u8; 24]);
        let pixels = render(size, base);
        for row in pixels.chunks(size).rev() {
            for pixel in row {
                image.extend(pixel.to_le_bytes()); // B, G, R, A
            }
        }
        image.extend(vec![0u8; mask_row * size]);
        images.push((size, image));
    }
    let mut file = Vec::new();
    file.extend(
        [0u16, 1, images.len() as u16]
            .iter()
            .flat_map(|v| v.to_le_bytes()),
    );
    let mut offset = 6 + 16 * images.len();
    for (size, image) in &images {
        let side = if *size >= 256 { 0 } else { *size as u8 };
        file.extend([side, side, 0, 0]);
        file.extend(1u16.to_le_bytes());
        file.extend(32u16.to_le_bytes());
        file.extend((image.len() as u32).to_le_bytes());
        file.extend((offset as u32).to_le_bytes());
        offset += image.len();
    }
    for (_, image) in images {
        file.extend(image);
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_with_a_white_microphone() {
        let size = 32;
        let pixels = render(size, BLUE);
        let at = |x: usize, y: usize| pixels[y * size + x];
        assert_eq!(at(0, 0) >> 24, 0, "corner transparent");
        assert_eq!(
            at(size / 2, size * 5 / 16),
            0xFFFF_FFFF,
            "microphone head white"
        );
        assert_eq!(at(size / 8, size / 2), 0xFF28_68D0, "badge in status color");
    }

    #[test]
    fn ico_has_a_directory_entry_per_size() {
        let file = ico(&[16, 256], BLUE);
        assert_eq!(&file[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(file[6], 16, "first entry 16 px");
        assert_eq!(file[6 + 16], 0, "256 px is written as 0");
        let first = 6 + 2 * 16;
        assert_eq!(&file[first..first + 4], &40u32.to_le_bytes());
        assert_eq!(
            file.len(),
            first + (40 + 16 * 16 * 4 + 4 * 16) + (40 + 256 * 256 * 4 + 32 * 256)
        );
    }
}
