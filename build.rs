// Embeds the app icon in the exe, so Explorer and the Desktop show it.
include!("src/icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=src/icon.rs");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sizes = [16u32, 24, 32, 48, 64, 256];
    let images: Vec<Vec<u8>> = sizes.iter().map(|&n| bmp(n, &icon_rgba(n))).collect();
    // ICO: header, one 16-byte entry per size, then the images.
    let mut ico = vec![0, 0, 1, 0, sizes.len() as u8, 0];
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&n, img) in sizes.iter().zip(&images) {
        let side = if n >= 256 { 0 } else { n as u8 }; // 0 means 256
        ico.extend_from_slice(&[side, side, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in &images {
        ico.extend_from_slice(img);
    }
    std::fs::write(out.join("icon.ico"), ico).unwrap();
    std::fs::write(out.join("icon.rc"), "1 ICON \"icon.ico\"\n").unwrap();
    let obj = out.join("icon.o");
    let built = std::process::Command::new("windres").current_dir(&out).args(["icon.rc", "-O", "coff", "-o"]).arg(&obj).status();
    if built.is_ok_and(|s| s.success()) {
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    } else {
        println!("cargo:warning=windres (from the MinGW toolchain) wasn't found, so the exe has no icon");
    }
}

/// One ICO image: a 32-bit BMP without its file header, rows bottom-up, then an all-clear AND mask.
fn bmp(n: u32, rgba: &[u8]) -> Vec<u8> {
    let mask_row = n.div_ceil(32) * 4;
    let mut v = Vec::new();
    for x in [40, n, 2 * n] {
        v.extend_from_slice(&x.to_le_bytes()); // header size, width, height (image + mask)
    }
    v.extend_from_slice(&[1, 0, 32, 0]); // planes, bits per pixel
    for x in [0, n * n * 4 + mask_row * n, 0, 0, 0, 0] {
        v.extend_from_slice(&x.to_le_bytes()); // no compression, image size, unused
    }
    for row in rgba.chunks_exact(n as usize * 4).rev() {
        for p in row.as_chunks::<4>().0 {
            v.extend_from_slice(&[p[2], p[1], p[0], p[3]]); // BGRA
        }
    }
    v.resize(v.len() + (mask_row * n) as usize, 0);
    v
}
