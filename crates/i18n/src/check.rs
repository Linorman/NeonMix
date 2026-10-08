//! Shared build-time and CLI validation. All FTL inspection uses Fluent's AST.
use fluent_bundle::{FluentArgs, FluentResource, FluentValue, concurrent::FluentBundle};
use fluent_syntax::{ast, parser};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub fallback: String,
    pub locale: Vec<LocaleEntry>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocaleEntry {
    pub tag: String,
    pub native_name: String,
    pub direction: String,
    pub status: String,
    pub aliases: Vec<String>,
}
pub fn manifest_from_str(value: &str) -> Result<Manifest, String> {
    toml::from_str(value).map_err(|error| error.to_string())
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    fragments: Vec<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fragment {
    #[serde(default)]
    message: Vec<MessageSpec>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSpec {
    pub id: String,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
    #[serde(default)]
    pub attributes: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Catalog {
    pub manifest: Manifest,
    pub messages: Vec<MessageSpec>,
    pub resources: Vec<(String, PathBuf, String)>,
    pub inputs: Vec<PathBuf>,
}

pub fn load_catalog(root: &Path) -> Result<Catalog, String> {
    let manifest_path = root.join("locales/manifest.toml");
    let index_path = root.join("messages.toml");
    let read =
        |path: &Path| fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()));
    let manifest = manifest_from_str(&read(&manifest_path)?)?;
    let index: Index = toml::from_str(&read(&index_path)?).map_err(|e| e.to_string())?;
    if manifest.version != 1 || index.version != 1 {
        return Err("unsupported manifest/schema version".into());
    }
    if manifest.fallback != "en" {
        return Err("the complete final fallback must be en".into());
    }
    let mut seen_tags = BTreeSet::new();
    let mut seen_locale_variants = BTreeSet::new();
    let mut seen_aliases = BTreeSet::new();
    for entry in &manifest.locale {
        if !seen_tags.insert(&entry.tag) {
            return Err(format!("duplicate locale {}", entry.tag));
        }
        if entry.aliases.is_empty()
            || entry.native_name.is_empty()
            || !["ltr", "rtl"].contains(&entry.direction.as_str())
            || !["shipped", "draft"].contains(&entry.status.as_str())
        {
            return Err(format!("invalid locale metadata {}", entry.tag));
        }
        entry
            .tag
            .parse::<unic_langid::LanguageIdentifier>()
            .map_err(|e| format!("locale {}: {e}", entry.tag))?;
        let variant = variant_name(&entry.tag.to_ascii_lowercase());
        if variant == "Self" || !seen_locale_variants.insert(variant) {
            return Err(format!("locale Rust variant collision: {}", entry.tag));
        }
        for alias in &entry.aliases {
            alias
                .strip_suffix("-*")
                .unwrap_or(alias)
                .parse::<unic_langid::LanguageIdentifier>()
                .map_err(|e| format!("invalid locale alias {alias}: {e}"))?;
            if entry.status == "shipped" && !seen_aliases.insert(alias.to_ascii_lowercase()) {
                return Err(format!("duplicate shipped alias {alias}"));
            }
        }
    }
    if !manifest
        .locale
        .iter()
        .any(|e| e.tag == "en" && e.status == "shipped")
    {
        return Err("en fallback is not shipped".into());
    }
    let mut catalog = Catalog {
        manifest,
        messages: vec![],
        resources: vec![],
        inputs: vec![manifest_path, index_path],
    };
    let mut seen_ids = BTreeSet::new();
    let mut seen_variants = BTreeSet::new();
    let mut seen_fragments = BTreeSet::new();
    for fragment in index.fragments {
        let fragment_path = Path::new(&fragment);
        if fragment_path.is_absolute()
            || fragment_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || !seen_fragments.insert(fragment.clone())
        {
            return Err(format!("invalid/duplicate fragment {fragment}"));
        }
        let path = root.join(fragment_path);
        let parsed: Fragment =
            toml::from_str(&read(&path)?).map_err(|e| format!("{}: {e}", path.display()))?;
        catalog.inputs.push(path);
        for message in parsed.message {
            if !valid_identifier(&message.id) || !seen_ids.insert(message.id.clone()) {
                return Err(format!("invalid/duplicate schema ID {}", message.id));
            }
            for (name, kind) in &message.args {
                if !valid_identifier(name)
                    || name.contains('-')
                    || !["string", "message", "u64", "f64"].contains(&kind.as_str())
                {
                    return Err(format!("{}: invalid argument {name}:{kind}", message.id));
                }
            }
            let mut attributes = BTreeSet::new();
            for attribute in &message.attributes {
                if !valid_identifier(attribute) || !attributes.insert(attribute) {
                    return Err(format!(
                        "{}: invalid/duplicate attribute {attribute}",
                        message.id
                    ));
                }
            }
            for id in std::iter::once(message.id.clone()).chain(
                message
                    .attributes
                    .iter()
                    .map(|a| format!("{}.{a}", message.id)),
            ) {
                if !seen_variants.insert(variant_name(&id)) {
                    return Err(format!("Rust variant collision: {id}"));
                }
            }
            catalog.messages.push(message);
        }
        let module = fragment_path
            .file_stem()
            .ok_or_else(|| format!("invalid fragment {fragment}"))?
            .to_string_lossy();
        for locale in catalog
            .manifest
            .locale
            .iter()
            .filter(|entry| entry.status == "shipped")
        {
            let resource_path = root
                .join("locales")
                .join(&locale.tag)
                .join(format!("{module}.ftl"));
            let source = read(&resource_path)?;
            catalog.inputs.push(resource_path.clone());
            catalog
                .resources
                .push((locale.tag.clone(), resource_path, source));
        }
    }
    // A file outside the index would otherwise silently disappear from binaries.
    for entry in catalog
        .manifest
        .locale
        .iter()
        .filter(|e| e.status == "shipped")
    {
        for path in
            fs::read_dir(root.join("locales").join(&entry.tag)).map_err(|e| e.to_string())?
        {
            let path = path.map_err(|e| e.to_string())?.path();
            if path.extension().is_some_and(|e| e == "ftl") && !catalog.inputs.contains(&path) {
                return Err(format!("unindexed FTL resource {}", path.display()));
            }
        }
    }
    Ok(catalog)
}
fn valid_identifier(value: &str) -> bool {
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub fn variant_name(id: &str) -> String {
    id.split(['-', '.', '_'])
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
struct Node {
    variables: BTreeSet<String>,
    numeric: BTreeSet<String>,
    selectors: BTreeMap<String, BTreeSet<String>>,
    references: Vec<Reference>,
}
#[derive(Clone, Debug)]
struct Reference {
    target: String,
    bindings: BTreeMap<String, BTreeSet<String>>,
    string_bindings: BTreeSet<String>,
}
type Graph = BTreeMap<String, Node>;

/// Generate test-only expanded resources by transforming Fluent text AST
/// nodes. Identifiers, selectors, references and arguments remain unchanged.
pub fn generate_pseudo(catalog: &Catalog, destination: &Path) -> Result<usize, String> {
    let mut pseudo_catalog = catalog.clone();
    let mut generated = vec![];
    for (_, path, source) in pseudo_catalog
        .resources
        .iter_mut()
        .filter(|(tag, _, _)| tag == "en")
    {
        let mut ast = parser::parse(source.clone()).map_err(|(_, e)| format!("{e:?}"))?;
        for entry in &mut ast.body {
            match entry {
                ast::Entry::Message(message) => {
                    if let Some(value) = &mut message.value {
                        expand_pattern(value);
                    }
                    for attribute in &mut message.attributes {
                        expand_pattern(&mut attribute.value);
                    }
                }
                ast::Entry::Term(term) => {
                    expand_pattern(&mut term.value);
                    for attribute in &mut term.attributes {
                        expand_pattern(&mut attribute.value);
                    }
                }
                _ => {}
            }
        }
        let expanded = fluent_syntax::serializer::serialize(&ast);
        parser::parse(expanded.clone()).map_err(|(_, e)| format!("pseudo serialization: {e:?}"))?;
        *source = expanded.clone();
        generated.push((
            destination.join(
                path.file_name()
                    .ok_or_else(|| "missing filename".to_owned())?,
            ),
            expanded,
        ));
    }
    // Pseudolocalization must pass the exact same schema and representative
    // rendering contracts as shipped resources before writing its artifact.
    validate(&pseudo_catalog)?;
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    for (path, source) in &generated {
        fs::write(path, source).map_err(|e| e.to_string())?;
    }
    Ok(generated.len())
}
fn expand_pattern(value: &mut ast::Pattern<String>) {
    for element in &mut value.elements {
        match element {
            ast::PatternElement::TextElement { value } => {
                if value.trim().is_empty()
                    || ["NeonMix", "AirPlay", "PIN", "UUID", "dB", "Hz", "ms", "%"]
                        .contains(&value.trim())
                {
                    continue;
                }
                let padding = value.chars().count().div_ceil(2);
                let accented = value
                    .split_inclusive(|c: char| !c.is_alphabetic())
                    .map(|segment| {
                        let word = segment.trim_end_matches(|c: char| !c.is_alphabetic());
                        if [
                            "NeonMix", "AirPlay", "Hub", "Sender", "Mixer", "PIN", "UUID", "ID",
                            "RMS", "dBFS", "dB", "Hz", "ms", "Ctrl", "Auto", "English",
                        ]
                        .contains(&word)
                        {
                            segment.to_owned()
                        } else {
                            segment
                                .chars()
                                .map(|c| match c {
                                    'a' => 'á',
                                    'e' => 'ë',
                                    'i' => 'ï',
                                    'o' => 'ô',
                                    'u' => 'ü',
                                    'A' => 'Á',
                                    'E' => 'Ë',
                                    other => other,
                                })
                                .collect::<String>()
                        }
                    })
                    .collect::<String>();
                *value = format!("⟦{accented}{}⟧", "·".repeat(padding));
            }
            ast::PatternElement::Placeable { expression } => expand_expression(expression),
        }
    }
}
fn expand_expression(value: &mut ast::Expression<String>) {
    match value {
        ast::Expression::Select { variants, .. } => {
            for variant in variants {
                expand_pattern(&mut variant.value);
            }
        }
        ast::Expression::Inline(ast::InlineExpression::Placeable { expression }) => {
            expand_expression(expression)
        }
        _ => {}
    }
}

fn pattern(pattern: &ast::Pattern<String>, node: &mut Node) -> Result<(), String> {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression: expr } = element {
            expression(expr, node)?;
        }
    }
    Ok(())
}
fn expression(expr: &ast::Expression<String>, node: &mut Node) -> Result<(), String> {
    match expr {
        ast::Expression::Inline(value) => inline(value, node),
        ast::Expression::Select { selector, variants } => {
            inline(selector, node)?;
            if let ast::InlineExpression::VariableReference { id } = selector {
                for variant in variants {
                    let key = match &variant.key {
                        ast::VariantKey::Identifier { name } => name.clone(),
                        ast::VariantKey::NumberLiteral { value } => {
                            node.numeric.insert(id.name.clone());
                            value.clone()
                        }
                    };
                    if ["zero", "one", "two", "few", "many"].contains(&key.as_str()) {
                        node.numeric.insert(id.name.clone());
                    }
                    node.selectors
                        .entry(id.name.clone())
                        .or_default()
                        .insert(key);
                }
            }
            for variant in variants {
                pattern(&variant.value, node)?;
            }
            Ok(())
        }
    }
}
fn inline(value: &ast::InlineExpression<String>, node: &mut Node) -> Result<(), String> {
    match value {
        ast::InlineExpression::VariableReference { id } => {
            node.variables.insert(id.name.clone());
        }
        ast::InlineExpression::MessageReference { id, attribute } => {
            node.references.push(Reference {
                target: reference_id(&id.name, attribute),
                bindings: BTreeMap::new(),
                string_bindings: BTreeSet::new(),
            })
        }
        ast::InlineExpression::TermReference {
            id,
            attribute,
            arguments,
        } => {
            let mut bindings = BTreeMap::new();
            let mut string_bindings = BTreeSet::new();
            if let Some(args) = arguments {
                if !args.positional.is_empty() {
                    return Err("term positional arguments are not supported".into());
                }
                for argument in &args.named {
                    if matches!(argument.value, ast::InlineExpression::StringLiteral { .. }) {
                        string_bindings.insert(argument.name.name.clone());
                    }
                    let mut bound = Node::default();
                    inline(&argument.value, &mut bound)?;
                    if !bound.references.is_empty() {
                        return Err("term arguments must be literal values".into());
                    }
                    node.variables.extend(bound.variables.clone());
                    bindings.insert(argument.name.name.clone(), bound.variables);
                }
            }
            node.references.push(Reference {
                target: reference_id(&format!("-{}", id.name), attribute),
                bindings,
                string_bindings,
            });
        }
        // No locale-sensitive function has been integrated; parsing one must
        // never silently authorize a capability the renderer cannot execute.
        ast::InlineExpression::FunctionReference { id, .. } => {
            return Err(format!(
                "function {} is not in the allowed function registry (currently empty)",
                id.name
            ));
        }
        ast::InlineExpression::Placeable { expression: expr } => expression(expr, node)?,
        ast::InlineExpression::StringLiteral { .. }
        | ast::InlineExpression::NumberLiteral { .. } => {}
    }
    Ok(())
}
fn reference_id(id: &str, attribute: &Option<ast::Identifier<String>>) -> String {
    attribute
        .as_ref()
        .map_or_else(|| id.to_owned(), |a| format!("{id}.{}", a.name))
}

fn insert_node(graph: &mut Graph, id: String, value: &ast::Pattern<String>) -> Result<(), String> {
    let mut node = Node::default();
    pattern(value, &mut node).map_err(|e| format!("{id}: {e}"))?;
    if graph.insert(id.clone(), node).is_some() {
        return Err(format!("duplicate FTL key/attribute {id}"));
    }
    Ok(())
}
fn parse_graph(
    resources: &[(String, PathBuf, String)],
    tag: &str,
) -> Result<(Graph, FluentBundle<FluentResource>), String> {
    let mut graph = Graph::new();
    let mut base_ids = BTreeSet::new();
    let mut bundle = FluentBundle::new_concurrent(vec![tag.parse().map_err(|e| format!("{e}"))?]);
    for (_, path, source) in resources.iter().filter(|(locale, _, _)| locale == tag) {
        let resource = parser::parse(source.clone())
            .map_err(|(_, errors)| format!("{}: FTL syntax errors: {errors:?}", path.display()))?;
        for entry in &resource.body {
            match entry {
                ast::Entry::Message(message) => {
                    let id = &message.id.name;
                    if !base_ids.insert(id.clone()) {
                        return Err(format!("duplicate FTL key {id}"));
                    }
                    if let Some(value) = &message.value {
                        insert_node(&mut graph, id.clone(), value)?;
                    }
                    for attr in &message.attributes {
                        insert_node(&mut graph, format!("{id}.{}", attr.id.name), &attr.value)?;
                    }
                }
                ast::Entry::Term(term) => {
                    let id = format!("-{}", term.id.name);
                    if !base_ids.insert(id.clone()) {
                        return Err(format!("duplicate FTL term {id}"));
                    }
                    insert_node(&mut graph, id.clone(), &term.value)?;
                    for attr in &term.attributes {
                        insert_node(&mut graph, format!("{id}.{}", attr.id.name), &attr.value)?;
                    }
                }
                ast::Entry::Junk { .. } => return Err(format!("{}: FTL junk", path.display())),
                _ => {}
            }
        }
        let parsed = FluentResource::try_new(source.clone()).map_err(|(_, e)| format!("{e:?}"))?;
        bundle
            .add_resource(parsed)
            .map_err(|e| format!("{}: {e:?}", path.display()))?;
    }
    Ok((graph, bundle))
}

fn requirements(id: &str, graph: &Graph, stack: &mut BTreeSet<String>) -> Result<Node, String> {
    if !stack.insert(id.to_owned()) {
        return Err(format!("reference cycle at {id}"));
    }
    let mut node = graph
        .get(id)
        .ok_or_else(|| format!("unknown reference {id}"))?
        .clone();
    for reference in node.references.clone() {
        let child = requirements(&reference.target, graph, stack)?;
        for bound in reference.bindings.keys() {
            if !child.variables.contains(bound) {
                return Err(format!(
                    "{}: undeclared term argument {bound}",
                    reference.target
                ));
            }
            if child.numeric.contains(bound) && reference.string_bindings.contains(bound) {
                return Err(format!(
                    "{}: numeric term argument {bound} received string",
                    reference.target
                ));
            }
        }
        for variable in &child.variables {
            if let Some(binding) = reference.bindings.get(variable) {
                node.variables.extend(binding.clone());
                if child.numeric.contains(variable) {
                    node.numeric.extend(binding.clone());
                }
            } else {
                node.variables.insert(variable.clone());
            }
        }
        for variable in child.numeric {
            if !reference.bindings.contains_key(&variable) {
                node.numeric.insert(variable);
            }
        }
        for (variable, keys) in child.selectors {
            if let Some(binding) = reference.bindings.get(&variable) {
                for v in binding {
                    node.selectors
                        .entry(v.clone())
                        .or_default()
                        .extend(keys.clone());
                }
            } else {
                node.selectors.entry(variable).or_default().extend(keys);
            }
        }
    }
    stack.remove(id);
    Ok(node)
}

pub fn validate(catalog: &Catalog) -> Result<(), String> {
    for locale in catalog
        .manifest
        .locale
        .iter()
        .filter(|e| e.status == "shipped")
    {
        let (graph, bundle) = parse_graph(&catalog.resources, &locale.tag)?;
        let expected: BTreeSet<String> = catalog
            .messages
            .iter()
            .flat_map(|m| {
                std::iter::once(m.id.clone())
                    .chain(m.attributes.iter().map(|a| format!("{}.{a}", m.id)))
            })
            .collect();
        for id in graph.keys() {
            requirements(id, &graph, &mut BTreeSet::new())?;
            if !id.starts_with('-') && !expected.contains(id) {
                return Err(format!("{}: undeclared FTL key {id}", locale.tag));
            }
        }
        for message in &catalog.messages {
            for id in std::iter::once(message.id.clone()).chain(
                message
                    .attributes
                    .iter()
                    .map(|a| format!("{}.{a}", message.id)),
            ) {
                let required = requirements(&id, &graph, &mut BTreeSet::new())
                    .map_err(|e| format!("{}: {e}", locale.tag))?;
                let declared = message.args.keys().cloned().collect::<BTreeSet<_>>();
                if required.variables != declared {
                    return Err(format!(
                        "{}:{id}: parameter contract: expected {declared:?}, found {:?}",
                        locale.tag, required.variables
                    ));
                }
                for numeric in &required.numeric {
                    if message
                        .args
                        .get(numeric)
                        .is_some_and(|kind| kind == "string" || kind == "message")
                    {
                        return Err(format!(
                            "{}:{id}: numeric selector {numeric} requires u64/f64",
                            locale.tag
                        ));
                    }
                }
                render_representatives(&bundle, &id, message, &required)?;
            }
        }
    }
    Ok(())
}

fn render_representatives(
    bundle: &FluentBundle<FluentResource>,
    id: &str,
    spec: &MessageSpec,
    required: &Node,
) -> Result<(), String> {
    let (key, attribute) = id.split_once('.').map_or((id, None), |(k, a)| (k, Some(a)));
    let message = bundle
        .get_message(key)
        .ok_or_else(|| format!("missing message {id}"))?;
    let pattern = if let Some(attribute) = attribute {
        message.get_attribute(attribute).map(|a| a.value())
    } else {
        message.value()
    }
    .ok_or_else(|| format!("missing pattern {id}"))?;
    let mut samples: BTreeMap<String, Vec<FluentValue<'static>>> = BTreeMap::new();
    for (name, kind) in &spec.args {
        let values = match kind.as_str() {
            "string" | "message" => {
                let mut strings = vec!["设备 { $evil } <script> 😀 العربية".to_owned()];
                if let Some(keys) = required.selectors.get(name) {
                    strings.extend(keys.iter().cloned());
                }
                strings.into_iter().map(FluentValue::from).collect()
            }
            "u64" | "f64" => {
                let mut numbers = vec![0.0, 1.0, 2.0, 3.0, 5.0, 11.0, 21.0, 101.0];
                if kind == "f64" {
                    numbers.push(1.5);
                }
                if let Some(keys) = required.selectors.get(name) {
                    numbers.extend(keys.iter().filter_map(|key| key.parse::<f64>().ok()));
                }
                numbers.into_iter().map(FluentValue::from).collect()
            }
            _ => return Err(format!("invalid type {kind}")),
        };
        samples.insert(name.clone(), values);
    }
    // Vary each argument independently, reaching every declared selector
    // branch without constructing an unbounded Cartesian product.
    let attempts = std::iter::once(None).chain(
        samples
            .iter()
            .flat_map(|(name, values)| values.iter().map(move |v| Some((name, v)))),
    );
    for variation in attempts {
        let mut args = FluentArgs::new();
        for (name, values) in &samples {
            let value = variation
                .filter(|(n, _)| *n == name)
                .map_or(&values[0], |(_, v)| v);
            args.set(name, value.clone());
        }
        let mut errors = vec![];
        bundle.format_pattern(pattern, Some(&args), &mut errors);
        if !errors.is_empty() {
            return Err(format!("{id}: representative rendering failed: {errors:?}"));
        }
    }
    Ok(())
}
