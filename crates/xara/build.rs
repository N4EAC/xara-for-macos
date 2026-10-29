use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/audio/coreaudio.c");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") || env::var_os("CARGO_FEATURE_COREAUDIO").is_none() {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported macOS architecture {other}"),
    };
    let status = Command::new("clang")
        .args([
            "-arch",
            arch,
            "-mmacosx-version-min=12.0",
            "-std=c11",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-c",
            "src/audio/coreaudio.c",
            "-o",
        ])
        .arg(out.join("coreaudio.o"))
        .status()
        .expect("clang is required for Core Audio");
    assert!(status.success(), "Core Audio adapter failed to compile");
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(out.join("libxara_coreaudio.a"))
            .arg(out.join("coreaudio.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=xara_coreaudio");
    for framework in ["AudioToolbox", "CoreAudio", "CoreFoundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
