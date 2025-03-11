use core::{error::Error, result::Result};
use std::{env, path::PathBuf};

static BLACKLIST: [&str; 6] = [
    "libavcodec/d3d11va.h",
    "libavcodec/dxva2.h",
    "libavcodec/qsv.h",
    "libavcodec/vdpau.h",
    "libavcodec/videotoolbox.h",
    "libavcodec/xvmc.h",
];

fn main() -> Result<(), Box<dyn Error>> {
    let libdir_path = PathBuf::from("ffmpeg-build")
        .canonicalize()
        .expect("cannot canonicalize path");

    println!(
        "cargo:rustc-link-search={}/lib",
        libdir_path.to_str().unwrap()
    );

    // Newlib
    println!("cargo:rustc-link-search=/usr/arm-none-eabi/lib");
    println!("cargo:rustc-link-lib=c");
    println!("cargo:rustc-link-lib=m");

    println!("cargo:rustc-link-lib=static=avcodec");
    println!("cargo:rustc-link-lib=static=avformat");
    println!("cargo:rustc-link-lib=static=avutil");
    println!("cargo:rustc-link-lib=static=swscale");

    println!("cargo:rerun-if-changed=build.rs");

    // Search include dir and bindgen all headers
    let include_dir = libdir_path.join("include");

    let mut paths = Vec::new();
    let mut search = vec![include_dir.clone()];
    while let Some(search_dir) = search.pop() {
        for entry in std::fs::read_dir(search_dir)? {
            let path = entry?.path();
            if path.is_dir() {
                search.push(path);
            } else if !path.to_string_lossy().contains("hwcontext_")
                && !BLACKLIST
                    .contains(&path.strip_prefix(&include_dir)?.to_str().expect("shitface"))
            {
                // Make sure we've skipped any HW specific headers
                paths.push(path.clone());
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    let bindings = bindgen::builder()
        .headers(
            paths
                .into_iter()
                .map(|path| path.to_str().expect("shitface").to_owned()),
        )
        .clang_arg("--target=arm-none-eabi")
        .clang_arg(format!("-I{}", include_dir.to_string_lossy()))
        .clang_arg("-I/usr/arm-none-eabi/include")
        .prepend_enum_name(false)
        .use_core()
        .clang_macro_fallback()
        .generate()?;

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    Ok(())
}
