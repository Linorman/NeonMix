fn main() {
    // Project-local native runtime. Distribution relocation/bundling is E09.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../../.local/gstreamer/prefix/lib");
    }
}
