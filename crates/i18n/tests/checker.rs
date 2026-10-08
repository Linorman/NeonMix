use neonmix_i18n::check::{load_catalog, validate};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(en: &str, zh: &str, schema: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/tmp")
            .join(format!(
                "i18n-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(path.join("locales/en")).unwrap();
        fs::create_dir_all(path.join("locales/zh-CN")).unwrap();
        fs::create_dir_all(path.join("messages")).unwrap();
        fs::write(
            path.join("locales/manifest.toml"),
            include_str!("../locales/manifest.toml"),
        )
        .unwrap();
        fs::write(
            path.join("messages.toml"),
            "version=1\nfragments=[\"messages/test.toml\"]\n",
        )
        .unwrap();
        fs::write(path.join("messages/test.toml"), schema).unwrap();
        fs::write(path.join("locales/en/test.ftl"), en).unwrap();
        fs::write(path.join("locales/zh-CN/test.ftl"), zh).unwrap();
        Self(path)
    }
    fn check(&self) -> Result<(), String> {
        load_catalog(&self.0).and_then(|c| validate(&c))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn cli_rejects_invalid_catalog_with_nonzero_exit() {
    let fixture = Fixture::new(
        "test = { missing }\n",
        "test = Test\n",
        "[[message]]\nid=\"test\"\n",
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_i18n-check"))
        .arg(&fixture.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown reference"));
}

#[test]
fn pseudo_resources_preserve_ast_contracts() {
    let source = "test = { $count ->\n [one] One NeonMix AirPlay { $count } device\n *[other] Many { $count } devices\n }\n";
    let fixture = Fixture::new(
        source,
        source,
        "[[message]]\nid=\"test\"\nargs={count=\"u64\"}\n",
    );
    let catalog = load_catalog(&fixture.0).unwrap();
    let destination = fixture.0.join("pseudo");
    assert_eq!(
        neonmix_i18n::check::generate_pseudo(&catalog, &destination).unwrap(),
        1
    );
    let generated = fs::read_to_string(destination.join("test.ftl")).unwrap();
    assert!(generated.contains("$count"));
    assert!(generated.contains("⟦"));
    assert!(generated.contains("NeonMix AirPlay"));
    fs::write(fixture.0.join("locales/en/test.ftl"), generated).unwrap();
    fixture.check().unwrap();
}

#[test]
fn destructive_mutations_are_rejected() {
    let schema = "[[message]]\nid=\"test\"\nargs={name=\"string\"}\n";
    let valid = "test = Hello { $name }\n";
    assert!(Fixture::new(valid, valid, schema).check().is_ok());
    for invalid in [
        "",
        "test = Hello { $renamed }\n",
        "test = One\ntest = Two\n",
        "test = { test }\n",
        "test = { unknown }\n",
        "test = { NUMBER($name) }\n",
        "test = {\n",
    ] {
        let result = Fixture::new(invalid, valid, schema).check();
        assert!(result.is_err(), "mutation passed: {invalid}");
    }
}
#[test]
fn transitive_parameters_and_cycles_are_checked() {
    let schema = "[[message]]\nid=\"outer\"\nargs={name=\"string\"}\n[[message]]\nid=\"inner\"\nargs={name=\"string\"}\n";
    let valid = "outer = { inner }\ninner = { $name }\n";
    assert!(Fixture::new(valid, valid, schema).check().is_ok());
    let missing = schema.replace("id=\"outer\"\nargs={name=\"string\"}", "id=\"outer\"");
    assert!(
        Fixture::new(valid, valid, &missing)
            .check()
            .unwrap_err()
            .contains("parameter contract")
    );
    assert!(
        Fixture::new("outer = { inner }\ninner = { outer }\n", valid, schema)
            .check()
            .unwrap_err()
            .contains("cycle")
    );
}
#[test]
fn terms_attributes_and_numeric_contracts_are_checked() {
    let schema = "[[message]]\nid=\"test\"\nargs={count=\"u64\"}\nattributes=[\"a11y\"]\n";
    let valid = "-device = device\ntest = { $count ->\n [one] { $count } { -device }\n *[other] { $count } devices\n }\n .a11y = { $count } devices\n";
    assert!(Fixture::new(valid, valid, schema).check().is_ok());
    assert!(
        Fixture::new(valid, valid, &schema.replace("u64", "string"))
            .check()
            .unwrap_err()
            .contains("numeric selector")
    );
    assert!(
        Fixture::new(
            "-one = { -two }\n-two = { -one }\ntest = { $count }\n .a11y = { $count }\n",
            valid,
            schema
        )
        .check()
        .unwrap_err()
        .contains("cycle")
    );
    assert!(
        Fixture::new("test = { $count }\n", valid, schema)
            .check()
            .is_err()
    );
}

#[test]
fn term_literal_argument_contract_is_checked() {
    let schema = "[[message]]\nid=\"test\"\n";
    let valid = "-items = { $count ->\n [one] One item\n *[other] Many items\n }\ntest = { -items(count: 1) }\n";
    assert!(Fixture::new(valid, valid, schema).check().is_ok());
    assert!(
        Fixture::new(&valid.replace("count: 1", "count: \"one\""), valid, schema)
            .check()
            .unwrap_err()
            .contains("numeric term argument")
    );
    assert!(
        Fixture::new(
            "-brand = NeonMix\ntest = { -brand(unused: 1) }\n",
            "test = Brand\n",
            schema
        )
        .check()
        .unwrap_err()
        .contains("undeclared term argument")
    );
}
