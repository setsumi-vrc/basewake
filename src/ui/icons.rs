//! Icons drawn in code (no image files to ship): a small base station with a coloured lens.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Every managed station is on.
    On,
    /// Some on, or an operation is running.
    Partial,
    /// Everything off / unknown.
    Off,
    /// The last operation had failures.
    Error,
}

impl Tone {
    fn lens(self) -> [u8; 3] {
        match self {
            Tone::On => [0x3D, 0xD6, 0x8C],
            Tone::Partial => [0xF2, 0xB1, 0x3A],
            Tone::Off => [0x8A, 0x93, 0x9E],
            Tone::Error => [0xE5, 0x48, 0x4D],
        }
    }
}

/// RGBA pixels of a `size`×`size` icon.
pub fn rgba(tone: Tone, size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut px = vec![0u8; (size * size * 4) as usize];
    let body = [0x22, 0x26, 0x2E];
    let rim = [0x4A, 0x52, 0x5E];
    let lens = tone.lens();
    // Body: rounded square covering most of the icon.
    let (x0, y0, x1, y1, r) = (s * 0.08, s * 0.08, s * 0.92, s * 0.92, s * 0.2);
    let (cx, cy, lens_r) = (s * 0.5, s * 0.5, s * 0.26);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let body_cov = rounded_rect_coverage(fx, fy, x0, y0, x1, y1, r);
            if body_cov <= 0.0 {
                continue;
            }
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let lens_cov = (lens_r - d + 0.5).clamp(0.0, 1.0);
            let rim_cov = (lens_r + s * 0.07 - d + 0.5).clamp(0.0, 1.0);
            let mut c = mix(body, rim, rim_cov);
            c = mix(c, lens, lens_cov);
            let i = ((y * size + x) * 4) as usize;
            px[i..i + 3].copy_from_slice(&c);
            px[i + 3] = (body_cov * 255.0) as u8;
        }
    }
    px
}

/// The tray glyph: a light base-station body on a transparent background, its lens lit in the
/// state colour (dark when everything is off), so it reads on light and dark taskbars alike.
pub fn tray_rgba(tone: Tone, size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut px = vec![0u8; (size * size * 4) as usize];
    let body = [0xD3, 0xD7, 0xDD];
    let ring = [0x1C, 0x1F, 0x24];
    let lens = match tone {
        Tone::Off => [0x3A, 0x3E, 0x46],
        t => t.lens(),
    };
    // The body is a touch wider than tall, like the real station's front face.
    let (x0, y0, x1, y1, r) = (s * 0.06, s * 0.12, s * 0.94, s * 0.88, s * 0.22);
    let (cx, cy) = (s * 0.5, s * 0.5);
    let (lens_r, ring_r) = (s * 0.2, s * 0.29);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let body_cov = rounded_rect_coverage(fx, fy, x0, y0, x1, y1, r);
            if body_cov <= 0.0 {
                continue;
            }
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let ring_cov = (ring_r - d + 0.5).clamp(0.0, 1.0);
            let lens_cov = (lens_r - d + 0.5).clamp(0.0, 1.0);
            let c = mix(mix(body, ring, ring_cov), lens, lens_cov);
            let i = ((y * size + x) * 4) as usize;
            px[i..i + 3].copy_from_slice(&c);
            px[i + 3] = (body_cov * 255.0) as u8;
        }
    }
    px
}

fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

fn rounded_rect_coverage(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> f32 {
    let qx = (x0 + r - x).max(x - (x1 - r)).max(0.0);
    let qy = (y0 + r - y).max(y - (y1 - r)).max(0.0);
    let outside = (qx * qx + qy * qy).sqrt() - r;
    let edge = (x0 - x).max(x - x1).max(y0 - y).max(y - y1);
    (0.5 - outside.max(edge)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_opaque_center_and_clear_corner() {
        let px = rgba(Tone::On, 32);
        assert_eq!(px.len(), 32 * 32 * 4);
        let center = ((16 * 32 + 16) * 4) as usize;
        assert_eq!(px[center + 3], 255);
        assert_eq!(&px[center..center + 3], &Tone::On.lens());
        assert_eq!(px[3], 0, "corner must be transparent");
    }
}
