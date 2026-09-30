//! `Info.plist` generation for `.driver` bundles.
//!
//! `coreaudiod` discovers an AudioServerPlugin by reading the
//! `Info.plist` inside its `.driver` bundle. Beyond the standard
//! `CFBundle*` keys, the plist must carry the CFPlugIn registration
//! dictionaries:
//!
//! - `CFPlugInFactories` maps a *factory UUID* to the name of the
//!   `#[no_mangle] extern "C"` factory function (the one
//!   [`plugin_entry!`](crate::plugin_entry) emits).
//! - `CFPlugInTypes` maps the well-known AudioServerPlugin *type
//!   UUID* ([`AUDIO_SERVER_PLUGIN_TYPE_UUID`]) to the list of
//!   factory UUIDs that implement it.
//!
//! [`generate`] emits a complete, `plutil`-valid XML plist from a
//! [`BundleConfig`]. The generator is opinionated — it covers the
//! keys an AudioServerPlugin needs and nothing else — and is
//! intended to be run from a `build.rs` or a packaging script in the
//! consuming driver crate.

extern crate alloc;

use alloc::string::String;
use core::fmt::Write as _;

/// `kAudioServerPlugInTypeUUID` — the CFPlugIn *type* UUID every
/// AudioServerPlugin implements. `coreaudiod` looks for factories
/// registered under this UUID in `CFPlugInTypes`.
///
/// Defined in `<CoreAudio/AudioServerPlugIn.h>`.
pub const AUDIO_SERVER_PLUGIN_TYPE_UUID: &str = "443ABAB8-E7B3-491A-B985-BEB9187030DB";

/// Static configuration for one `.driver` bundle's `Info.plist`.
///
/// Every field is `'static` — the generated plist is a `String`
/// returned by value, so the inputs need only outlive the
/// [`generate`] call.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct BundleConfig {
    /// `CFBundleIdentifier` — reverse-DNS bundle id, e.g.
    /// `"com.example.MyDriver"`.
    pub bundle_identifier: &'static str,
    /// `CFBundleName` — short display name for the bundle.
    pub bundle_name: &'static str,
    /// `CFBundleExecutable` — the file name of the cdylib inside
    /// `Contents/MacOS/`. Conventionally matches the `.driver`
    /// bundle's base name.
    pub executable: &'static str,
    /// `CFBundleVersion` / `CFBundleShortVersionString` — the
    /// plug-in version, e.g. `"0.1.0"`.
    pub version: &'static str,
    /// The CFPlugIn *factory* UUID for this driver. Must be a
    /// freshly-generated UUID, unique to the driver; it is the key
    /// `coreaudiod` uses to find the factory function. Format:
    /// canonical upper-case `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX`.
    pub factory_uuid: &'static str,
    /// The name of the `#[no_mangle] extern "C"` CFPlugIn factory
    /// function — the symbol [`plugin_entry!`](crate::plugin_entry)
    /// exports. `coreaudiod` resolves this symbol in the executable
    /// and calls it to create the plug-in.
    pub factory_function: &'static str,
}

impl BundleConfig {
    /// Begin a bundle configuration with the CFPlugIn registration
    /// fields, defaulting the remaining `CFBundle*` keys so they can
    /// be overridden with the builder-style setters.
    ///
    /// Defaults: `bundle_name` and `executable` both fall back to
    /// `bundle_identifier`, and `version` to `"0.0.0"`.
    #[must_use]
    pub const fn new(
        bundle_identifier: &'static str,
        factory_uuid: &'static str,
        factory_function: &'static str,
    ) -> Self {
        Self {
            bundle_identifier,
            bundle_name: bundle_identifier,
            executable: bundle_identifier,
            version: "0.0.0",
            factory_uuid,
            factory_function,
        }
    }

    /// Builder-style: set `CFBundleName`.
    #[must_use]
    pub const fn with_bundle_name(mut self, name: &'static str) -> Self {
        self.bundle_name = name;
        self
    }

    /// Builder-style: set `CFBundleExecutable`.
    #[must_use]
    pub const fn with_executable(mut self, executable: &'static str) -> Self {
        self.executable = executable;
        self
    }

    /// Builder-style: set `CFBundleVersion` /
    /// `CFBundleShortVersionString`.
    #[must_use]
    pub const fn with_version(mut self, version: &'static str) -> Self {
        self.version = version;
        self
    }
}

/// Generate the complete `Info.plist` XML for a `.driver` bundle.
///
/// The output covers:
///
/// - The standard `CFBundle*` keys (`CFBundleIdentifier`,
///   `CFBundleName`, `CFBundleExecutable`, `CFBundleVersion`,
///   `CFBundleShortVersionString`, `CFBundlePackageType = "BNDL"`,
///   `CFBundleInfoDictionaryVersion`).
/// - `CFPlugInDynamicRegistration = "NO"` — the plug-in registers
///   statically through the dictionaries below.
/// - `CFPlugInFactories` — `{ factory_uuid: factory_function }`.
/// - `CFPlugInTypes` — `{ AUDIO_SERVER_PLUGIN_TYPE_UUID:
///   [ factory_uuid ] }`.
///
/// The result is a well-formed XML property list that `plutil
/// -lint` accepts. String values are XML-escaped, so identifiers and
/// names containing `&`, `<`, or `>` are emitted safely.
#[must_use]
pub fn generate(config: &BundleConfig) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
    );
    out.push_str("<plist version=\"1.0\">\n");
    out.push_str("<dict>\n");

    key_string(&mut out, "CFBundleInfoDictionaryVersion", "6.0");
    key_string(&mut out, "CFBundleIdentifier", config.bundle_identifier);
    key_string(&mut out, "CFBundleName", config.bundle_name);
    key_string(&mut out, "CFBundleExecutable", config.executable);
    key_string(&mut out, "CFBundleVersion", config.version);
    key_string(&mut out, "CFBundleShortVersionString", config.version);
    key_string(&mut out, "CFBundlePackageType", "BNDL");
    // CFPlugInDynamicRegistration is a CFBoolean-as-string in the
    // AudioServerPlugin convention; `coreaudiod` reads the string
    // form, matching Apple's `SimpleAudioDriver` sample.
    key_string(&mut out, "CFPlugInDynamicRegistration", "NO");

    // CFPlugInFactories: { factory_uuid: factory_function }
    indent(&mut out, 1);
    out.push_str("<key>CFPlugInFactories</key>\n");
    indent(&mut out, 1);
    out.push_str("<dict>\n");
    indent(&mut out, 2);
    let _ = writeln!(out, "<key>{}</key>", escape_xml(config.factory_uuid));
    indent(&mut out, 2);
    let _ = writeln!(
        out,
        "<string>{}</string>",
        escape_xml(config.factory_function)
    );
    indent(&mut out, 1);
    out.push_str("</dict>\n");

    // CFPlugInTypes: { AUDIO_SERVER_PLUGIN_TYPE_UUID: [factory_uuid] }
    indent(&mut out, 1);
    out.push_str("<key>CFPlugInTypes</key>\n");
    indent(&mut out, 1);
    out.push_str("<dict>\n");
    indent(&mut out, 2);
    let _ = writeln!(
        out,
        "<key>{}</key>",
        escape_xml(AUDIO_SERVER_PLUGIN_TYPE_UUID)
    );
    indent(&mut out, 2);
    out.push_str("<array>\n");
    indent(&mut out, 3);
    let _ = writeln!(out, "<string>{}</string>", escape_xml(config.factory_uuid));
    indent(&mut out, 2);
    out.push_str("</array>\n");
    indent(&mut out, 1);
    out.push_str("</dict>\n");

    out.push_str("</dict>\n");
    out.push_str("</plist>\n");
    out
}

/// Write a `<key>…</key>\n<string>…</string>\n` pair at one level
/// of indentation, XML-escaping the value.
fn key_string(out: &mut String, key: &str, value: &str) {
    indent(out, 1);
    let _ = writeln!(out, "<key>{}</key>", escape_xml(key));
    indent(out, 1);
    let _ = writeln!(out, "<string>{}</string>", escape_xml(value));
}

/// Append `level` tab stops (one tab each) to `out`.
fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push('\t');
    }
}

/// Escape the five XML predefined entities so arbitrary identifier
/// and name strings embed safely in the plist body.
fn escape_xml(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> BundleConfig {
        BundleConfig::new(
            "com.example.MinimalLoopback",
            "9E5B7C2A-1D3F-4A6B-8C9D-0E1F2A3B4C5D",
            "TympanAsplDriverFactory",
        )
        .with_bundle_name("Minimal Loopback")
        .with_executable("MinimalLoopback")
        .with_version("0.1.0")
    }

    #[test]
    fn new_defaults_optional_fields_to_the_identifier() {
        let c = BundleConfig::new("com.example.Foo", "UUID", "Factory");
        assert_eq!(c.bundle_name, "com.example.Foo");
        assert_eq!(c.executable, "com.example.Foo");
        assert_eq!(c.version, "0.0.0");
    }

    #[test]
    fn builder_setters_override_defaults() {
        let c = sample_config();
        assert_eq!(c.bundle_name, "Minimal Loopback");
        assert_eq!(c.executable, "MinimalLoopback");
        assert_eq!(c.version, "0.1.0");
    }

    #[test]
    fn generated_plist_has_xml_and_doctype_preamble() {
        let plist = generate(&sample_config());
        assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(plist.contains("<!DOCTYPE plist PUBLIC"));
        assert!(plist.contains("<plist version=\"1.0\">"));
        assert!(plist.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn generated_plist_embeds_all_cfbundle_keys() {
        let plist = generate(&sample_config());
        for key in [
            "CFBundleInfoDictionaryVersion",
            "CFBundleIdentifier",
            "CFBundleName",
            "CFBundleExecutable",
            "CFBundleVersion",
            "CFBundleShortVersionString",
            "CFBundlePackageType",
            "CFPlugInDynamicRegistration",
            "CFPlugInFactories",
            "CFPlugInTypes",
        ] {
            assert!(
                plist.contains(&format!("<key>{key}</key>")),
                "missing key {key} in:\n{plist}"
            );
        }
    }

    #[test]
    fn generated_plist_embeds_identity_and_factory_values() {
        let plist = generate(&sample_config());
        assert!(plist.contains("<string>com.example.MinimalLoopback</string>"));
        assert!(plist.contains("<string>Minimal Loopback</string>"));
        assert!(plist.contains("<string>MinimalLoopback</string>"));
        assert!(plist.contains("<string>0.1.0</string>"));
        assert!(plist.contains("<string>BNDL</string>"));
        assert!(plist.contains("<string>TympanAsplDriverFactory</string>"));
        // The factory UUID appears twice: once as the
        // CFPlugInFactories key, once in the CFPlugInTypes array.
        assert_eq!(
            plist
                .matches("9E5B7C2A-1D3F-4A6B-8C9D-0E1F2A3B4C5D")
                .count(),
            2
        );
    }

    #[test]
    fn generated_plist_wires_the_audioserver_type_uuid() {
        let plist = generate(&sample_config());
        assert!(plist.contains(&format!("<key>{AUDIO_SERVER_PLUGIN_TYPE_UUID}</key>")));
        // The type UUID's value is an <array> of factory UUIDs.
        assert!(plist.contains("<array>"));
        assert!(plist.contains("</array>"));
    }

    #[test]
    fn audioserver_type_uuid_matches_core_audio() {
        assert_eq!(
            AUDIO_SERVER_PLUGIN_TYPE_UUID,
            "443ABAB8-E7B3-491A-B985-BEB9187030DB"
        );
    }

    #[test]
    fn string_values_are_xml_escaped() {
        let config = BundleConfig::new("com.example.A&B", "UUID", "Factory<dangerous>")
            .with_bundle_name("Quote\"Test");
        let plist = generate(&config);
        assert!(plist.contains("com.example.A&amp;B"));
        assert!(plist.contains("Factory&lt;dangerous&gt;"));
        assert!(plist.contains("Quote&quot;Test"));
        // The raw, unescaped forms must not leak through.
        assert!(!plist.contains("A&B"));
        assert!(!plist.contains("<dangerous>"));
    }

    #[test]
    fn escape_xml_handles_all_five_entities() {
        assert_eq!(
            escape_xml("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
        assert_eq!(escape_xml("nothing to escape"), "nothing to escape");
    }

    #[test]
    fn key_value_pairs_are_balanced() {
        // Every <key> should be followed by exactly one value; a
        // cheap structural check is that <key> and <string> /
        // <dict> / <array> opening tags pair up. Count the simplest
        // invariant: balanced <dict> tags.
        let plist = generate(&sample_config());
        assert_eq!(
            plist.matches("<dict>").count(),
            plist.matches("</dict>").count()
        );
        assert_eq!(
            plist.matches("<array>").count(),
            plist.matches("</array>").count()
        );
    }
}
