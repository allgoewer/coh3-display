//! The app's icon, drawn in code: the display itself, a dark panel with a yellow line above
//! blue ones. The exe's icon (`examples/make_icon.rs`) and the tray icon share the drawing.

type Rgb = [u8; 3];

const FRAME: Rgb = [0x5c, 0x66, 0x72];
const PANEL: Rgb = [0x1b, 0x20, 0x26];
/// The two colours of the real OLED: its top rows are yellow, the rest is blue.
const YELLOW: Rgb = [0xf2, 0xc2, 0x30];
const BLUE: Rgb = [0x4c, 0xc2, 0xff];
/// The lines of a display that is not showing anything.
const UNLIT: Rgb = [0x4a, 0x52, 0x5c];

/// A rectangle with rounded corners, in a 64-unit square with the origin at the top left.
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
}

impl Rect {
    fn contains(&self, px: f32, py: f32) -> bool {
        // Distance to the rectangle that is left when the corner radius is taken off.
        let dx = ((px - (self.x + self.w / 2.0)).abs() - (self.w / 2.0 - self.r)).max(0.0);
        let dy = ((py - (self.y + self.h / 2.0)).abs() - (self.h / 2.0 - self.r)).max(0.0);
        dx * dx + dy * dy <= self.r * self.r
    }
}

/// The drawing, back to front. `small` is a plainer version that survives 16 pixels.
fn layers(small: bool, lit: bool) -> Vec<(Rect, Rgb)> {
    let rect = |x, y, w, h, r| Rect { x, y, w, h, r };
    let (top, rest) = if lit { (YELLOW, BLUE) } else { (UNLIT, UNLIT) };
    let border = if small { 3.5 } else { 1.5 };
    let mut layers = vec![
        (rect(4.0, 8.0, 56.0, 48.0, 9.0), FRAME),
        (rect(4.0 + border, 8.0 + border, 56.0 - 2.0 * border, 48.0 - 2.0 * border, 9.0 - border), PANEL),
    ];
    if small {
        layers.extend([
            (rect(12.0, 16.0, 40.0, 8.0, 0.0), top),
            (rect(12.0, 28.0, 40.0, 8.0, 0.0), rest),
            (rect(12.0, 40.0, 24.0, 8.0, 0.0), rest),
        ]);
    } else {
        layers.extend([
            (rect(12.0, 16.0, 30.0, 7.0, 2.0), top),
            (rect(46.0, 17.5, 6.0, 4.0, 1.0), top),
            (rect(12.0, 28.0, 40.0, 7.0, 2.0), rest),
            (rect(12.0, 40.0, 26.0, 7.0, 2.0), rest),
        ]);
    }
    layers
}

/// Samples per pixel along each axis; what smooths the edges.
const SAMPLES: u32 = 4;

/// The icon at `size` x `size` pixels as RGBA rows, top to bottom. `lit` is the look for
/// "a display is showing the match"; otherwise its lines are grey.
pub fn render(size: u32, lit: bool) -> Vec<u8> {
    let layers = layers(size < 24, lit);
    let unit = 64.0 / (size * SAMPLES) as f32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0u32; 3];
            let mut hits = 0;
            for sample in 0..SAMPLES * SAMPLES {
                let px = ((x * SAMPLES + sample % SAMPLES) as f32 + 0.5) * unit;
                let py = ((y * SAMPLES + sample / SAMPLES) as f32 + 0.5) * unit;
                if let Some((_, colour)) = layers.iter().rev().find(|(rect, _)| rect.contains(px, py)) {
                    for (total, channel) in sum.iter_mut().zip(colour) {
                        *total += u32::from(*channel);
                    }
                    hits += 1;
                }
            }
            let colour = sum.map(|total| (total / hits.max(1)) as u8);
            rgba.extend_from_slice(&colour);
            rgba.push((hits * 255 / (SAMPLES * SAMPLES)) as u8);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(rgba: &[u8]) -> Vec<u8> {
        rgba.as_chunks::<4>().0.iter().map(|pixel| pixel[3]).collect()
    }

    #[test]
    fn fills_the_middle_and_leaves_the_corners_clear() {
        for size in [16u32, 32, 256] {
            let rgba = render(size, true);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            let alpha = alpha(&rgba);
            assert_eq!(alpha[0], 0, "at {size}");
            assert_eq!(alpha[(size * size - 1) as usize], 0, "at {size}");
            assert_eq!(alpha[(size / 2 * size + size / 2) as usize], 255, "at {size}");
        }
    }

    #[test]
    fn an_unlit_display_has_the_same_shape_in_grey() {
        for size in [16, 48] {
            let (lit, unlit) = (render(size, true), render(size, false));
            assert_ne!(lit, unlit, "at {size}");
            assert_eq!(alpha(&lit), alpha(&unlit), "at {size}");
        }
        // The top line is the yellow one.
        let lit = render(64, true);
        let pixel = (19 * 64 + 20) * 4;
        assert_eq!(lit[pixel..pixel + 3], YELLOW);
    }
}
