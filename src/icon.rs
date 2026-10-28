// std only: build.rs includes this file to embed the same icon in the exe.

/// The app icon, an accent-purple disc with a white play triangle, as `n`×`n` RGBA.
/// Drawn on a 32-unit grid with 4×4 supersampling, so it's smooth at any size.
pub fn icon_rgba(n: u32) -> Vec<u8> {
    let s = n as f32 / 32.0;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let (mut disc, mut play) = (0.0f32, 0.0f32);
            for i in 0..4 {
                for j in 0..4 {
                    let fx = (x as f32 + (i as f32 + 0.5) / 4.0) / s;
                    let fy = (y as f32 + (j as f32 + 0.5) / 4.0) / s;
                    if (fx - 16.0).powi(2) + (fy - 16.0).powi(2) <= 15.5f32.powi(2) {
                        disc += 1.0 / 16.0;
                        // Play triangle: (12,9) (12,23) (24,16)
                        if fx >= 12.0 && (fy - 16.0).abs() <= (24.0 - fx) * 7.0 / 12.0 {
                            play += 1.0 / 16.0;
                        }
                    }
                }
            }
            let t = if disc > 0.0 { play / disc } else { 0.0 };
            let mix = |purple: f32| (purple + (255.0 - purple) * t).round() as u8;
            px.extend_from_slice(&[mix(100.0), mix(80.0), mix(240.0), (disc * 255.0).round() as u8]);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_is_round_with_a_white_play_triangle_at_any_size() {
        for n in [32u32, 256] {
            let px = icon_rgba(n);
            assert_eq!(px.len(), (n * n * 4) as usize);
            assert_eq!(px[3], 0); // corner is transparent
            let centre = (((n / 2) * n + n / 2) * 4) as usize;
            assert_eq!(&px[centre..centre + 4], &[255, 255, 255, 255]);
            let left = (((n / 2) * n + n / 8) * 4) as usize; // left of the triangle, inside the disc
            assert_eq!(&px[left..left + 4], &[100, 80, 240, 255]);
        }
    }
}
