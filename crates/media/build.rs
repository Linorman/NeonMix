fn main() {
    // Cargo rewrites DYLD_FALLBACK_LIBRARY_PATH for test executables. Keep
    // native media tests runnable using only the extracted project runtime.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../../../.local/gstreamer/prefix/lib"
        );
    }
}
