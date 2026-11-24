//! Build steps: copy the OpenVR runtime library next to the executables and, on Windows, give
//! the exe an icon.
use std::path::PathBuf;

#[path = "src/ui/icons.rs"]
#[allow(dead_code)]
mod icons;

fn main() {
    println!("cargo:rerun-if-changed=src/ui/icons.rs");
    // Build scripts run on the host, so ask Cargo which system the binaries are for.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let lib = match target_os.as_str() {
        "windows" => Some("openvr_api.dll"),
        "linux" => Some("libopenvr_api.so"),
        _ => None, // no SteamVR on macOS
    };
    if let Some(lib) = lib {
        let src = PathBuf::from("third_party/openvr-2.12.14").join(lib);
        println!("cargo:rerun-if-changed={}", src.display());
        // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out → target/<profile> is 3 levels up.
        if let Some(profile_dir) = out.ancestors().nth(3) {
            let _ = std::fs::copy(&src, profile_dir.join(lib));
        }
    }

    if target_os == "windows" {
        let ico = out.join("basewake.ico");
        std::fs::write(&ico, ico_file(&[16, 32, 48, 256])).unwrap();
        let mut res = winresource::WindowsResource::new();
        res.set_icon(ico.to_str().unwrap());
        res.set("FileDescription", "Basewake");
        res.set("ProductName", "Basewake");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed icon: {e}");
        }
    }
}

/// A multi-size .ico with 32-bit BMP images.
fn ico_file(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| bmp_entry(s)).collect();
    let mut f = Vec::new();
    f.extend_from_slice(&[0, 0, 1, 0]);
    f.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let dim = if *s >= 256 { 0 } else { *s as u8 };
        f.extend_from_slice(&[dim, dim, 0, 0, 1, 0, 32, 0]);
        f.extend_from_slice(&(img.len() as u32).to_le_bytes());
        f.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        f.extend_from_slice(&img);
    }
    f
}

fn bmp_entry(size: u32) -> Vec<u8> {
    let rgba = icons::rgba(icons::Tone::On, size);
    let mut b = Vec::new();
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(size as i32).to_le_bytes());
    b.extend_from_slice(&((size * 2) as i32).to_le_bytes()); // XOR + AND masks
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(&[0u8; 24]);
    for y in (0..size).rev() {
        for x in 0..size {
            let i = ((y * size + x) * 4) as usize;
            b.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
        }
    }
    let mask_row = size.div_ceil(32) * 4;
    b.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
    b
}
