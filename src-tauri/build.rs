fn main() {
    // ScreenCaptureKit's Swift bridge links the Swift runtime via @rpath; link args set by a
    // dependency's build script don't reach our binary, so add the system location here.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
    tauri_build::build()
}
