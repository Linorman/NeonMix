//! Print the `Info.plist` the `bundle` module generates for the
//! `minimal-loopback` example driver.
//!
//! Tier 2 CI runs this and pipes the output through `plutil -lint`,
//! so the `bundle::plist` generator is verified to emit a plist
//! `coreaudiod`'s loader would accept. The configuration here is
//! kept in sync with `examples/minimal-loopback/Info.plist`.
//!
//! ```bash
//! cargo run --example print-info-plist
//! ```

use tympan_aspl::bundle::plist::{generate, BundleConfig};

fn main() {
    let config = BundleConfig::new(
        "com.tympan.aspl.MinimalLoopback",
        "9E5B7C2A-1D3F-4A6B-8C9D-0E1F2A3B4C5D",
        "TympanAsplDriverFactory",
    )
    .with_bundle_name("Minimal Loopback")
    .with_executable("MinimalLoopback")
    .with_version("0.1.0");

    // `print!`, not `println!`: `generate` already terminates the
    // plist with a trailing newline.
    print!("{}", generate(&config));
}
