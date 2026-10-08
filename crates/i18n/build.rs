#[allow(dead_code)]
#[path = "src/check.rs"]
mod check;

use std::{env, fs, path::PathBuf};

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    // Directory watches also detect newly added, unindexed resources.
    println!("cargo:rerun-if-changed=messages");
    println!("cargo:rerun-if-changed=locales");
    println!("cargo:rerun-if-changed=messages.toml");
    let catalog = check::load_catalog(&root).unwrap_or_else(|e| panic!("i18n catalog: {e}"));
    for input in &catalog.inputs {
        println!("cargo:rerun-if-changed={}", input.display());
    }
    check::validate(&catalog).unwrap_or_else(|e| panic!("i18n contract: {e}"));
    let locales = catalog
        .manifest
        .locale
        .iter()
        .filter(|entry| entry.status == "shipped")
        .collect::<Vec<_>>();
    let mut generated = String::from(
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum TextDirection { Ltr, Rtl }\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum ResolvedLocale {\n",
    );
    for entry in &locales {
        generated.push_str(&format!(
            "{},\n",
            check::variant_name(&entry.tag.to_ascii_lowercase())
        ));
    }
    generated.push_str(
        "}\nimpl ResolvedLocale {\npub const fn tag(self) -> &'static str { match self {\n",
    );
    for entry in &locales {
        generated.push_str(&format!(
            "Self::{} => {:?},\n",
            check::variant_name(&entry.tag.to_ascii_lowercase()),
            entry.tag
        ));
    }
    generated.push_str("} }\npub const fn native_name(self) -> &'static str { match self {\n");
    for entry in &locales {
        generated.push_str(&format!(
            "Self::{} => {:?},\n",
            check::variant_name(&entry.tag.to_ascii_lowercase()),
            entry.native_name
        ));
    }
    generated.push_str("} }\npub const fn direction(self) -> TextDirection { match self {\n");
    for entry in &locales {
        generated.push_str(&format!(
            "Self::{} => TextDirection::{},\n",
            check::variant_name(&entry.tag.to_ascii_lowercase()),
            if entry.direction == "ltr" {
                "Ltr"
            } else {
                "Rtl"
            }
        ));
    }
    generated.push_str("} }\npub const fn shipped() -> &'static [Self] { &[\n");
    for entry in &locales {
        generated.push_str(&format!(
            "Self::{},\n",
            check::variant_name(&entry.tag.to_ascii_lowercase())
        ));
    }
    generated.push_str("] }\npub fn from_tag(tag: &str) -> Option<Self> { match tag {\n");
    for entry in &locales {
        generated.push_str(&format!(
            "{:?} => Some(Self::{}),\n",
            entry.tag,
            check::variant_name(&entry.tag.to_ascii_lowercase())
        ));
    }
    generated
        .push_str("_ => None,\n} }\n}\n#[derive(Clone, Debug, PartialEq)]\npub enum Message {\n");
    let mut metadata = String::new();
    let mut arguments = String::new();
    let mut representatives = String::new();
    for message in &catalog.messages {
        for id in std::iter::once(message.id.clone()).chain(
            message
                .attributes
                .iter()
                .map(|a| format!("{}.{a}", message.id)),
        ) {
            let variant = check::variant_name(&id);
            let fields = message
                .args
                .iter()
                .map(|(name, kind)| {
                    format!(
                        "{name}: {}",
                        match kind.as_str() {
                            "string" => "String",
                            "message" => "Box<Message>",
                            other => other,
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let pattern = if fields.is_empty() {
                format!("Self::{variant}")
            } else {
                format!(
                    "Self::{variant} {{ {} }}",
                    message.args.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            };
            generated.push_str(&format!(
                "    {variant}{} ,\n",
                if fields.is_empty() {
                    String::new()
                } else {
                    format!(" {{ {fields} }}")
                }
            ));
            metadata.push_str(&format!(
                "{} => {:?},\n",
                if fields.is_empty() {
                    format!("Self::{variant}")
                } else {
                    format!("Self::{variant} {{ .. }}")
                },
                id
            ));
            arguments.push_str(&format!("{pattern} => {{\n"));
            for (name, kind) in &message.args {
                let value = if kind == "message" {
                    format!("localizer.render({name})")
                } else if kind == "string" {
                    format!("{name}.as_str()")
                } else {
                    format!("*{name}")
                };
                arguments.push_str(&format!("args.set({name:?}, {value});\n"));
            }
            arguments.push_str("}\n");
            for number in [0, 1, 2, 5, 11, 101] {
                let fields = message
                    .args
                    .iter()
                    .map(|(name, kind)| {
                        format!(
                            "{name}: {}",
                            match kind.as_str() {
                                "string" =>
                                    "String::from(\"设备 { $literal } 😀 العربية\")".to_owned(),
                                "message" => "Box::new(Message::CommonUnknown)".to_owned(),
                                "u64" => number.to_string(),
                                _ => format!("{number}.5"),
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                representatives.push_str(&format!(
                    "Message::{variant}{},\n",
                    if fields.is_empty() {
                        String::new()
                    } else {
                        format!(" {{ {fields} }}")
                    }
                ));
                if message.args.is_empty() {
                    break;
                }
            }
        }
    }
    generated.push_str("}\nimpl Message {\npub fn id(&self) -> &'static str { match self {\n");
    generated.push_str(&metadata);
    generated.push_str("} }\npub(crate) fn args(&self, #[allow(unused_variables)] localizer: &Localizer) -> fluent_bundle::FluentArgs<'_> {\n#[allow(unused_mut)] let mut args = fluent_bundle::FluentArgs::new();\nmatch self {\n");
    generated.push_str(&arguments);
    generated.push_str("}\nargs\n}\n}\npub fn representative_messages() -> Vec<Message> { vec![\n");
    generated.push_str(&representatives);
    generated.push_str("] }\npub(crate) const EMBEDDED_RESOURCES: &[(&str, &str)] = &[\n");
    for (tag, path, _) in &catalog.resources {
        generated.push_str(&format!(
            "({tag:?}, include_str!({:?})),\n",
            path.to_string_lossy()
        ));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory")).join("messages.rs"),
        generated,
    )
    .expect("write generated messages");
}
