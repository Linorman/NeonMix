//! Rust syntax-aware detection at presentation calls. This deliberately does
//! not classify machine logs, protocol keys, IDs or user data by language.
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Expr, Lit,
    parse::Parser,
    punctuated::Punctuated,
    visit::{self, Visit},
};

#[derive(Clone, Debug)]
pub struct Finding {
    pub file: PathBuf,
    pub line: usize,
    pub value: String,
    pub call: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exceptions {
    #[serde(default)]
    exception: Vec<Exception>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Exception {
    file: String,
    line: usize,
    value: String,
    category: String,
    reason: String,
}

struct LiteralCollector {
    literals: Vec<(usize, String, bool)>,
}
impl<'ast> Visit<'ast> for LiteralCollector {
    fn visit_expr_index(&mut self, expression: &'ast syn::ExprIndex) {
        // JSON/map keys identify data, rather than contributing visible text.
        self.visit_expr(&expression.expr);
    }
    fn visit_expr_if(&mut self, expression: &'ast syn::ExprIf) {
        self.visit_block(&expression.then_branch);
        if let Some((_, value)) = &expression.else_branch {
            self.visit_expr(value);
        }
    }
    fn visit_expr_match(&mut self, expression: &'ast syn::ExprMatch) {
        for arm in &expression.arms {
            self.visit_expr(&arm.body);
        }
    }
    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        self.visit_expr(&expression.receiver);
        if [
            "unwrap_or",
            "unwrap_or_else",
            "map_or",
            "map_or_else",
            "or_else",
        ]
        .contains(&expression.method.to_string().as_str())
        {
            for argument in &expression.args {
                self.visit_expr(argument);
            }
        }
    }
    fn visit_expr_lit(&mut self, literal: &'ast syn::ExprLit) {
        if let Lit::Str(value) = &literal.lit {
            self.literals
                .push((value.span().start().line, value.value(), false));
        }
    }
    fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
        if expression.mac.path.is_ident("format")
            && let Ok(args) = Punctuated::<Expr, syn::Token![,]>::parse_terminated
                .parse2(expression.mac.tokens.clone())
        {
            for (index, arg) in args.iter().enumerate() {
                if index == 0
                    && let Expr::Lit(literal) = arg
                    && let Lit::Str(value) = &literal.lit
                {
                    self.literals
                        .push((value.span().start().line, value.value(), true));
                } else {
                    self.visit_expr(arg);
                }
            }
        }
    }
}
struct Calls {
    findings: Vec<Finding>,
    path: PathBuf,
}
impl Calls {
    fn inspect(&mut self, expression: &Expr, call: &str) {
        let mut collector = LiteralCollector { literals: vec![] };
        collector.visit_expr(expression);
        for (line, value, formatted) in collector.literals {
            // Punctuation, numbers and unit-only patterns have no translation.
            // English and all other alphabetic scripts are equally inspected.
            let visible = if formatted {
                display_literal(&value)
            } else {
                value.clone()
            };
            if visible.chars().any(char::is_alphabetic) {
                self.findings.push(Finding {
                    file: self.path.clone(),
                    line,
                    value,
                    call: call.to_owned(),
                });
            }
        }
    }
}
impl<'ast> Visit<'ast> for Calls {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if module.attrs.iter().any(|attr| attr.path().is_ident("cfg") && matches!(&attr.meta, syn::Meta::List(list) if list.tokens.to_string().contains("test"))) { return; }
        visit::visit_item_mod(self, module);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        let index = match method.as_str() {
            "label"
            | "heading"
            | "small"
            | "strong"
            | "weak"
            | "button"
            | "link"
            | "on_hover_text"
            | "on_disabled_hover_text"
            | "hint_text"
            | "set_label"
            | "set_name"
            | "set_tooltip" => Some(0),
            "selected_text" | "monospace" | "hyperlink_to" | "with_tooltip" | "with_title"
            | "set_text" => Some(0),
            "colored_label" => Some(1),
            "text" => Some(2),
            "checkbox" | "radio" | "selectable_label" | "selectable_value" => {
                Some(if method == "selectable_value" { 2 } else { 1 })
            }
            _ => None,
        };
        if let Some(expression) = index.and_then(|i| call.args.get(i)) {
            self.inspect(expression, &method);
        }
        visit::visit_expr_method_call(self, call);
    }
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let Expr::Path(function) = call.func.as_ref() {
            let parts = function
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>();
            let name = parts.join("::");
            let last = parts.last().map(String::as_str).unwrap_or("");
            let index = if [
                "RichText::new",
                "Label::new",
                "Button::new",
                "CollapsingHeader::new",
                "Window::new",
                "Submenu::new",
            ]
            .iter()
            .any(|suffix| name.ends_with(suffix))
            {
                Some(0)
            } else if name.ends_with("MenuItem::with_id") {
                Some(1)
            } else if name.ends_with("WidgetInfo::labeled") {
                Some(2)
            } else if parts.iter().any(|p| p == "widgets") {
                match last {
                    "button" | "button_compact" | "caption" | "note" | "error_text" | "mono"
                    | "dot" => Some(1),
                    "button_enabled" | "small_button" => Some(2),
                    "button_busy" | "toggle" | "toggle_small" => Some(3),
                    "field" | "field_sized" | "title_field" => Some(2),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(expression) = index.and_then(|i| call.args.get(i)) {
                self.inspect(expression, &name);
            }
        }
        visit::visit_expr_call(self, call);
    }
}

// Rust's AST already identifies string literals and format macro arguments.
// Remove only interpolation specifications when deciding whether the literal
// contributes words; a template consisting solely of values/punctuation is
// not a hardcoded sentence. The Rust compiler validates format syntax itself.
fn display_literal(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                output.push('{');
            } else {
                for character in chars.by_ref() {
                    if character == '}' {
                        break;
                    }
                }
            }
        } else {
            output.push(character);
        }
    }
    output
}

pub fn scan_product_calls(project: &Path) -> Result<Vec<Finding>, String> {
    let mut paths = vec![];
    collect_rust(&project.join("apps/desktop/src"), &mut paths)?;
    let mut findings = vec![];
    for path in paths {
        let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let parsed =
            syn::parse_file(&source).map_err(|e| format!("{}: Rust parse: {e}", path.display()))?;
        let mut calls = Calls {
            findings: vec![],
            path: path
                .strip_prefix(project)
                .map_err(|e| e.to_string())?
                .to_owned(),
        };
        calls.visit_file(&parsed);
        findings.extend(calls.findings);
    }
    findings.sort_by(|a, b| (&a.file, a.line, &a.value).cmp(&(&b.file, b.line, &b.value)));
    findings.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.value == b.value);
    Ok(findings)
}

pub fn check_product_calls(project: &Path, exception_path: &Path) -> Result<Vec<Finding>, String> {
    let mut findings = scan_product_calls(project)?;
    let exceptions: Exceptions = if exception_path.exists() {
        toml::from_str(&fs::read_to_string(exception_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
    } else {
        Exceptions { exception: vec![] }
    };
    let mut used = vec![false; exceptions.exception.len()];
    findings.retain(|finding| {
        for (index, exception) in exceptions.exception.iter().enumerate() {
            if !exception.reason.trim().is_empty()
                && ["brand", "technical", "language-autonym", "user-content"]
                    .contains(&exception.category.as_str())
                && exception.file == finding.file.to_string_lossy().replace('\\', "/")
                && exception.line == finding.line
                && exception.value == finding.value
            {
                used[index] = true;
                return false;
            }
        }
        true
    });
    if let Some(index) = used.iter().position(|used| !used) {
        return Err(format!(
            "stale/invalid UI hardcoding exception at {}:{}",
            exceptions.exception[index].file, exceptions.exception[index].line
        ));
    }
    findings.sort_by(|a, b| (&a.file, a.line, &a.value).cmp(&(&b.file, b.line, &b.value)));
    findings.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.value == b.value);
    Ok(findings)
}
fn collect_rust(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect_rust(&path, files)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_chinese_english_and_formatted_presentation_literals() {
        let syntax = syn::parse_file("fn show() { ui.label(\"中文\"); ui.button(\"Save\"); widgets::note(ui, format!(\"Found {} devices\", count)); widgets::field(ui, \"stable-id\", &app.tr(Message::CommonSave), draft, false); } #[cfg(test)] mod tests { fn fixture() { ui.label(\"Fixture\"); } }").unwrap();
        let mut calls = Calls {
            findings: vec![],
            path: "fixture.rs".into(),
        };
        calls.visit_file(&syntax);
        assert_eq!(calls.findings.len(), 3);
        assert!(calls.findings.iter().any(|f| f.value == "Save"));
        assert!(!calls.findings.iter().any(|f| f.value == "stable-id"));
    }
    #[test]
    fn formatted_values_and_data_keys_are_distinct_from_display_words() {
        let syntax = syn::parse_file("fn show() { ui.label(format!(\"{title}: {value}\")); ui.label(source[\"name\"].as_str().unwrap_or(\"Unknown\")); ui.label(if app.pending(\"send\") { \"Working\" } else { \"Done\" }); ui.label(\"{ $name }\"); }").unwrap();
        let mut calls = Calls {
            findings: vec![],
            path: "fixture.rs".into(),
        };
        calls.visit_file(&syntax);
        let values = calls
            .findings
            .iter()
            .map(|f| f.value.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            values,
            std::collections::BTreeSet::from(["Unknown", "Working", "Done", "{ $name }"])
        );
    }
}
