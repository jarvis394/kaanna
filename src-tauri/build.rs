use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // ScreenCaptureKit's Swift bridge links the Swift runtime via @rpath; link args set by a
        // dependency's build script don't reach our binary, so add the system location here.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

        // whisper.cpp's Metal backend uses `@available`, which clang lowers to calls into its own
        // runtime (`__isPlatformVersionAtLeast`). rustc links with -nodefaultlibs, so that runtime
        // is missing unless we add it. Only the members actually needed are pulled in.
        if let Some(runtime) = clang_runtime() {
            println!("cargo:rustc-link-arg={}", runtime.display());
        }
    }
    tauri_build::build()
}

fn clang_runtime() -> Option<PathBuf> {
    let out = Command::new("xcrun").args(["clang", "--print-resource-dir"]).output().ok()?;
    let dir = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim());
    let lib = dir.join("lib/darwin/libclang_rt.osx.a");
    lib.exists().then_some(lib)
}
