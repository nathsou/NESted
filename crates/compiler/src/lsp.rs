//! Language server (LSP 3.17 subset) shared by the native binary and WASM.
//!
//! Documents are grouped into projects: the nearest directory with a
//! `main.nst`. Files that are not open can be supplied by a host loader
//! (the native server reads them from disk; the playground opens them).
use crate::check::{Analysis, Def, Project, BUILTIN_FNS};
use crate::hir::{GlobalKind, Value};
use crate::json::Json;
use crate::lexer::{Tok, KEYWORDS};
use crate::source::Span;
use crate::types::Type;
use std::collections::{BTreeMap, BTreeSet};

fn obj<const N: usize>(v: [(&str, Json); N]) -> Json {
    Json::object(v)
}

pub type Loader = Box<dyn Fn(&str) -> Vec<(String, Vec<u8>)>>;

struct Doc {
    text: String,
    version: Json,
}

struct Workspace {
    root: String,
    analysis: Analysis,
    /// Relative path → URI for every source file in the project.
    uris: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct LanguageServer {
    docs: BTreeMap<String, Doc>,
    assets: BTreeMap<String, Vec<u8>>,
    pub loader: Option<Loader>,
    cache: Option<Workspace>,
    published: BTreeSet<String>,
    pub exit: bool,
    shutdown: bool,
}

const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "function",
    "variable",
    "parameter",
    "keyword",
    "number",
    "string",
    "operator",
    "enumMember",
    "property",
    "macro",
];

fn dir_of(uri: &str) -> &str {
    match uri.rfind('/') {
        Some(i) => &uri[..i + 1],
        None => "",
    }
}

impl LanguageServer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Provide an asset for analysis (the playground's project files).
    pub fn set_asset(&mut self, uri: &str, data: Vec<u8>) {
        self.assets.insert(uri.to_owned(), data);
        self.cache = None;
    }

    pub fn text(&self, uri: &str) -> Option<&str> {
        self.docs.get(uri).map(|d| d.text.as_str())
    }

    /// The project root for a document: the closest ancestor with main.nst.
    fn root_of(&self, uri: &str) -> String {
        let mut dir = dir_of(uri).to_owned();
        loop {
            let main = format!("{dir}main.nst");
            if self.docs.contains_key(&main) {
                return dir;
            }
            if let Some(load) = &self.loader {
                if load(&main).iter().any(|(u, _)| *u == main) {
                    return dir;
                }
            }
            let trimmed = dir.trim_end_matches('/');
            match trimmed.rfind('/') {
                Some(i)
                    if i > 0 && !trimmed[..i].ends_with(':') && !trimmed[..i].ends_with("//") =>
                {
                    dir = trimmed[..i + 1].to_owned()
                }
                _ => return dir_of(uri).to_owned(),
            }
        }
    }

    fn workspace(&mut self, uri: &str) -> &Workspace {
        let root = self.root_of(uri);
        let valid = self.cache.as_ref().is_some_and(|w| w.root == root);
        if !valid {
            let mut project = Project::default();
            let mut uris = BTreeMap::new();
            if let Some(load) = &self.loader {
                for (u, bytes) in load(&root) {
                    if let Some(rel) = u.strip_prefix(&root) {
                        if rel.ends_with(".nst") {
                            project.sources.insert(
                                rel.to_owned(),
                                String::from_utf8_lossy(&bytes).into_owned(),
                            );
                            uris.insert(rel.to_owned(), u.clone());
                        } else {
                            project.assets.insert(rel.to_owned(), bytes);
                        }
                    }
                }
            }
            for (u, d) in &self.docs {
                if let Some(rel) = u.strip_prefix(&root) {
                    project.sources.insert(rel.to_owned(), d.text.clone());
                    uris.insert(rel.to_owned(), u.clone());
                }
            }
            for (u, bytes) in &self.assets {
                if let Some(rel) = u.strip_prefix(&root) {
                    project.assets.insert(rel.to_owned(), bytes.clone());
                }
            }
            let entry = if project.sources.contains_key("main.nst") {
                "main.nst".to_owned()
            } else {
                uri.strip_prefix(&root).unwrap_or("main.nst").to_owned()
            };
            project.entry = entry;
            let analysis = crate::check::analyze(&project);
            self.cache = Some(Workspace {
                root,
                analysis,
                uris,
            });
        }
        self.cache.as_ref().unwrap()
    }

    fn notification(method: &str, params: Json) -> Json {
        obj([
            ("jsonrpc", "2.0".into()),
            ("method", method.into()),
            ("params", params),
        ])
    }
    fn response(id: Json, result: Json) -> Json {
        obj([("jsonrpc", "2.0".into()), ("id", id), ("result", result)])
    }
    fn failure(id: Json, code: i32, message: &str) -> Json {
        obj([
            ("jsonrpc", "2.0".into()),
            ("id", id),
            (
                "error",
                obj([
                    ("code", Json::Number(code as f64)),
                    ("message", message.into()),
                ]),
            ),
        ])
    }

    fn publish(&mut self, uri: &str) -> Vec<Json> {
        let ws = self.workspace(uri);
        let a = &ws.analysis;
        let mut by_file: BTreeMap<String, Vec<Json>> = BTreeMap::new();
        for (rel, u) in &ws.uris {
            let _ = rel;
            by_file.entry(u.clone()).or_default();
        }
        for d in &a.diagnostics.items {
            let file = a.sources.get(d.span.file);
            let target = match ws.uris.get(&file.path) {
                Some(u) => u.clone(),
                None => {
                    // Problems inside the standard library are reported at the entry.
                    match ws.uris.get("main.nst") {
                        Some(u) => u.clone(),
                        None => continue,
                    }
                }
            };
            let range = if ws.uris.contains_key(&file.path) {
                span_range(a, d.span)
            } else {
                zero_range()
            };
            let message = if ws.uris.contains_key(&file.path) {
                d.message.clone()
            } else {
                format!("{}: {}", file.path, d.message)
            };
            by_file.entry(target).or_default().push(obj([
                ("range", range),
                (
                    "severity",
                    Json::Number(match d.severity {
                        crate::Severity::Error => 1.0,
                        crate::Severity::Warning => 2.0,
                        crate::Severity::Hint => 4.0,
                    }),
                ),
                ("source", "nested".into()),
                ("message", message.into()),
            ]));
        }
        let mut out = Vec::new();
        let mut now = BTreeSet::new();
        for (u, list) in by_file {
            now.insert(u.clone());
            out.push(Self::notification(
                "textDocument/publishDiagnostics",
                obj([("uri", u.into()), ("diagnostics", Json::Array(list))]),
            ));
        }
        for old in self.published.difference(&now) {
            out.push(Self::notification(
                "textDocument/publishDiagnostics",
                obj([
                    ("uri", old.clone().into()),
                    ("diagnostics", Json::Array(vec![])),
                ]),
            ));
        }
        self.published = now;
        out
    }

    pub fn handle(&mut self, request: Json) -> Vec<Json> {
        let method = request.get("method").str().to_owned();
        let id = request.get("id").clone();
        let params = request.get("params").clone();
        match method.as_str() {
            "initialize" => return vec![Self::response(id, capabilities())],
            "initialized"
            | "$/cancelRequest"
            | "workspace/didChangeConfiguration"
            | "textDocument/didSave"
            | "$/setTrace" => return vec![],
            "shutdown" => {
                self.shutdown = true;
                return vec![Self::response(id, Json::Null)];
            }
            "exit" => {
                self.exit = true;
                return vec![];
            }
            "textDocument/didOpen" => {
                let d = params.get("textDocument");
                let uri = d.get("uri").str().to_owned();
                self.docs.insert(
                    uri.clone(),
                    Doc {
                        text: d.get("text").str().to_owned(),
                        version: d.get("version").clone(),
                    },
                );
                self.cache = None;
                return self.publish(&uri);
            }
            "textDocument/didChange" => {
                let d = params.get("textDocument");
                let uri = d.get("uri").str().to_owned();
                let Some(doc) = self.docs.get_mut(&uri) else {
                    return vec![];
                };
                for change in params.get("contentChanges").array() {
                    if change.get("range").is_null() {
                        doc.text = change.get("text").str().to_owned();
                    } else {
                        let file = crate::source::SourceFile::new("", doc.text.clone());
                        let r = change.get("range");
                        let s = file.offset(
                            r.get("start").get("line").usize(),
                            r.get("start").get("character").usize(),
                        );
                        let e = file.offset(
                            r.get("end").get("line").usize(),
                            r.get("end").get("character").usize(),
                        );
                        if s <= e && e <= doc.text.len() {
                            doc.text.replace_range(s..e, change.get("text").str());
                        }
                    }
                }
                doc.version = d.get("version").clone();
                self.cache = None;
                return self.publish(&uri);
            }
            "textDocument/didClose" => {
                let uri = params.get("textDocument").get("uri").str().to_owned();
                // Keep the text: other files of the project still import it.
                let _ = uri;
                return vec![];
            }
            "nested/asset" => {
                // { uri, data: [bytes] }
                let uri = params.get("uri").str().to_owned();
                let data: Vec<u8> = params
                    .get("data")
                    .array()
                    .iter()
                    .map(|b| b.usize() as u8)
                    .collect();
                self.set_asset(&uri, data);
                return vec![];
            }
            _ => {}
        }
        if id.is_null() {
            return vec![];
        }
        if self.shutdown {
            return vec![Self::failure(id, -32600, "Server has shut down")];
        }
        if method == "nested/stdSource" {
            let path = params.get("path").str();
            let text = crate::check::std_source(path).unwrap_or("");
            return vec![Self::response(id, text.into())];
        }
        let uri = params.get("textDocument").get("uri").str().to_owned();
        if !self.docs.contains_key(&uri) {
            return vec![Self::failure(id, -32602, "Document is not open")];
        }
        let text = self.docs[&uri].text.clone();
        let result = match method.as_str() {
            "textDocument/formatting" => {
                let opts = params.get("options");
                let indent =
                    if opts.get("insertSpaces").bool() || opts.get("insertSpaces").is_null() {
                        " ".repeat(
                            opts.get("tabSize")
                                .usize()
                                .clamp(1, 8)
                                .max(if opts.get("tabSize").is_null() { 4 } else { 1 }),
                        )
                    } else {
                        "\t".into()
                    };
                let formatted =
                    crate::fmt::format(&text, &crate::fmt::Style { indent, width: 100 });
                if formatted == text {
                    Json::Array(vec![])
                } else {
                    let lines = text.lines().count() + 1;
                    Json::Array(vec![obj([
                        ("range", obj([("start", pos(0, 0)), ("end", pos(lines, 0))])),
                        ("newText", formatted.into()),
                    ])])
                }
            }
            _ => {
                let ws = self.workspace(&uri);
                let rel = uri.strip_prefix(&ws.root).unwrap_or("").to_owned();
                let a = &ws.analysis;
                let Some(file) = a.sources.find(&rel) else {
                    return vec![Self::response(id, Json::Null)];
                };
                let src = a.sources.get(file);
                let p = params.get("position");
                let offset = src.offset(p.get("line").usize(), p.get("character").usize());
                let at = a
                    .refs
                    .iter()
                    .filter(|(s, _)| s.file == file && s.contains(offset))
                    .min_by_key(|(s, _)| s.end - s.start)
                    .copied()
                    .or_else(|| {
                        a.defs
                            .iter()
                            .find(|(_, s)| s.file == file && s.contains(offset))
                            .map(|(d, s)| (*s, *d))
                    });
                match method.as_str() {
                    "textDocument/hover" => match at {
                        Some((span, def)) => obj([
                            (
                                "contents",
                                obj([
                                    ("kind", "markdown".into()),
                                    ("value", describe(a, def).into()),
                                ]),
                            ),
                            ("range", span_range(a, span)),
                        ]),
                        None => Json::Null,
                    },
                    "textDocument/definition" => {
                        match at.and_then(|(_, d)| a.defs.get(&d).map(|s| (d, *s))) {
                            Some((_, span)) => location(ws, a, span).unwrap_or(Json::Null),
                            None => Json::Null,
                        }
                    }
                    "textDocument/references" => match at {
                        Some((_, def)) => {
                            let mut out = Vec::new();
                            if let Some(s) = a.defs.get(&def) {
                                out.extend(location(ws, a, *s));
                            }
                            for (s, d) in &a.refs {
                                if *d == def {
                                    out.extend(location(ws, a, *s));
                                }
                            }
                            Json::Array(out)
                        }
                        None => Json::Array(vec![]),
                    },
                    "textDocument/prepareRename" => match at {
                        Some((span, def)) if !matches!(def, Def::Builtin(_) | Def::Module(_)) => {
                            span_range(a, span)
                        }
                        _ => Json::Null,
                    },
                    "textDocument/rename" => {
                        let new_name = params.get("newName").str().to_owned();
                        let valid = new_name
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                            && new_name
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                            && !KEYWORDS.contains(&new_name.as_str());
                        match at {
                            Some((_, def))
                                if valid && !matches!(def, Def::Builtin(_) | Def::Module(_)) =>
                            {
                                let mut spans: Vec<Span> = a
                                    .refs
                                    .iter()
                                    .filter(|(_, d)| *d == def)
                                    .map(|(s, _)| *s)
                                    .collect();
                                if let Some(s) = a.defs.get(&def) {
                                    spans.push(*s);
                                }
                                spans.sort();
                                spans.dedup();
                                let mut changes: BTreeMap<String, Vec<Json>> = BTreeMap::new();
                                for s in spans {
                                    let path = &a.sources.get(s.file).path;
                                    if let Some(u) = ws.uris.get(path) {
                                        changes.entry(u.clone()).or_default().push(obj([
                                            ("range", span_range(a, s)),
                                            ("newText", new_name.clone().into()),
                                        ]));
                                    }
                                }
                                obj([(
                                    "changes",
                                    Json::Object(
                                        changes
                                            .into_iter()
                                            .map(|(k, v)| (k, Json::Array(v)))
                                            .collect(),
                                    ),
                                )])
                            }
                            _ if !valid => {
                                return vec![Self::failure(id, -32602, "Not a valid name")]
                            }
                            _ => Json::Null,
                        }
                    }
                    "textDocument/completion" => completion(a, file, &text, offset),
                    "textDocument/signatureHelp" => signature(a, file, &text, offset),
                    "textDocument/semanticTokens/full" => semantic_tokens(a, file, ws),
                    "textDocument/inlayHint" => Json::Array(
                        a.hints
                            .iter()
                            .filter(|(s, _)| s.file == file)
                            .map(|(s, t)| {
                                let (l, c) = src.position(s.end as usize);
                                obj([
                                    ("position", pos(l, c)),
                                    ("label", format!(": {}", a.program.types.name(t)).into()),
                                    ("kind", Json::Number(1.0)),
                                    ("paddingLeft", false.into()),
                                ])
                            })
                            .collect(),
                    ),
                    "textDocument/documentSymbol" => document_symbols(a, file),
                    _ => return vec![Self::failure(id, -32601, "Method not found")],
                }
            }
        };
        vec![Self::response(id, result)]
    }
}

fn pos(line: usize, character: usize) -> Json {
    obj([("line", line.into()), ("character", character.into())])
}

fn zero_range() -> Json {
    obj([("start", pos(0, 0)), ("end", pos(0, 1))])
}

fn span_range(a: &Analysis, s: Span) -> Json {
    let f = a.sources.get(s.file);
    let (l1, c1) = f.position(s.start as usize);
    let (l2, c2) = f.position(s.end as usize);
    obj([("start", pos(l1, c1)), ("end", pos(l2, c2))])
}

fn location(ws: &Workspace, a: &Analysis, s: Span) -> Option<Json> {
    let path = &a.sources.get(s.file).path;
    let uri = match ws.uris.get(path) {
        Some(u) => u.clone(),
        None if path.starts_with("std/") => format!("nested-std:/{path}"),
        None => return None,
    };
    Some(obj([("uri", uri.into()), ("range", span_range(a, s))]))
}

fn capabilities() -> Json {
    obj([
        (
            "capabilities",
            obj([
                ("positionEncoding", "utf-16".into()),
                (
                    "textDocumentSync",
                    obj([
                        ("openClose", true.into()),
                        ("change", Json::Number(2.0)),
                        ("save", true.into()),
                    ]),
                ),
                ("hoverProvider", true.into()),
                ("definitionProvider", true.into()),
                ("referencesProvider", true.into()),
                ("renameProvider", obj([("prepareProvider", true.into())])),
                ("documentSymbolProvider", true.into()),
                ("documentFormattingProvider", true.into()),
                ("inlayHintProvider", true.into()),
                (
                    "completionProvider",
                    obj([(
                        "triggerCharacters",
                        Json::Array(vec![".".into(), ":".into(), "@".into()]),
                    )]),
                ),
                (
                    "signatureHelpProvider",
                    obj([(
                        "triggerCharacters",
                        Json::Array(vec!["(".into(), ",".into()]),
                    )]),
                ),
                (
                    "semanticTokensProvider",
                    obj([
                        (
                            "legend",
                            obj([
                                (
                                    "tokenTypes",
                                    Json::Array(
                                        TOKEN_TYPES.iter().map(|t| Json::from(*t)).collect(),
                                    ),
                                ),
                                (
                                    "tokenModifiers",
                                    Json::Array(vec![
                                        "declaration".into(),
                                        "readonly".into(),
                                        "defaultLibrary".into(),
                                    ]),
                                ),
                            ]),
                        ),
                        ("full", true.into()),
                    ]),
                ),
            ]),
        ),
        (
            "serverInfo",
            obj([
                ("name", "NESted".into()),
                ("version", env!("CARGO_PKG_VERSION").into()),
            ]),
        ),
    ])
}

/// Markdown description of a definition.
pub fn describe(a: &Analysis, def: Def) -> String {
    let p = &a.program;
    let t = |ty: &Type| p.types.name(ty);
    let code = |s: String, doc: &str| {
        if doc.is_empty() {
            format!("```nested\n{s}\n```")
        } else {
            format!("```nested\n{s}\n```\n\n{doc}")
        }
    };
    match def {
        Def::Global(g) => {
            let gl = p.global(g);
            let kw = match gl.kind {
                GlobalKind::Const => "const",
                GlobalKind::Var => "var",
                GlobalKind::Io(_) => "var",
            };
            let mut s = format!("{kw} {}: {}", gl.path, t(&gl.ty));
            if let (GlobalKind::Const, Value::Int(n)) = (&gl.kind, &gl.value) {
                s.push_str(&format!(" = {n}"));
            }
            if let GlobalKind::Io(addr) = gl.kind {
                s = format!("@io(0x{addr:04X}) {s}");
            }
            let size = p.types.size(&gl.ty);
            code(
                s,
                &format!(
                    "{}{}{size} byte{}",
                    gl.doc,
                    if gl.doc.is_empty() { "" } else { "\n\n" },
                    if size == 1 { "" } else { "s" }
                ),
            )
        }
        Def::Fn(f) => {
            let func = p.function(f);
            let params: Vec<String> = func
                .params
                .iter()
                .map(|l| {
                    let d = &func.locals[l.0 as usize];
                    format!(
                        "{}{}: {}",
                        if d.mutable { "var " } else { "" },
                        d.name,
                        t(&d.ty)
                    )
                })
                .collect();
            let ret = if func.result == Type::Void {
                String::new()
            } else {
                format!(" -> {}", t(&func.result))
            };
            let doc = doc_for(a, def);
            code(
                format!("fn {}({}){ret}", func.path, params.join(", ")),
                &doc,
            )
        }
        Def::Local(f, l) => {
            let func = p.function(f);
            match func.locals.get(l.0 as usize) {
                Some(d) => code(
                    format!(
                        "{} {}: {}",
                        if d.mutable { "var" } else { "let" },
                        d.name,
                        t(&d.ty)
                    ),
                    "",
                ),
                None => String::new(),
            }
        }
        Def::Struct(s) => {
            let d = &p.types.structs[s.0 as usize];
            let fields: Vec<String> = d
                .fields
                .iter()
                .map(|(n, ft)| format!("    {n}: {},", t(ft)))
                .collect();
            code(
                format!("struct {} {{\n{}\n}}", d.name, fields.join("\n")),
                &format!(
                    "{} bytes; arrays of it are stored field by field",
                    p.types.size(&Type::Struct(s))
                ),
            )
        }
        Def::Field(s, i) => {
            let d = &p.types.structs[s.0 as usize];
            let (n, ft) = &d.fields[i as usize];
            code(format!("{}.{n}: {}", d.name, t(ft)), "")
        }
        Def::Enum(e) => {
            let d = &p.types.enums[e.0 as usize];
            let v: Vec<String> = d
                .variants
                .iter()
                .map(|(n, x)| format!("    {n} = {x},"))
                .collect();
            code(
                format!("enum {}: {} {{\n{}\n}}", d.name, t(&d.repr), v.join("\n")),
                &doc_for(a, def),
            )
        }
        Def::Variant(e, i) => {
            let d = &p.types.enums[e.0 as usize];
            let (n, x) = &d.variants[i as usize];
            code(format!("{}::{n} = {x}", d.name), "")
        }
        Def::Range(r) => {
            let d = &p.types.ranges[r.0 as usize];
            code(
                format!("type {} = {} in {}..={}", d.name, t(&d.base), d.lo, d.hi),
                "Values are proven to stay in range; use it to index arrays without checks.",
            )
        }
        Def::Alias(i) => match a.aliases.get(i as usize) {
            Some((n, ty)) => code(format!("type {n} = {}", t(ty)), ""),
            None => String::new(),
        },
        Def::Module(m) => {
            let path = &a.modules[m as usize].path;
            let doc: Vec<String> = a.modules[m as usize]
                .comments
                .iter()
                .take_while(|c| c.text.starts_with("//!"))
                .map(|c| c.text.trim_start_matches("//!").trim().to_owned())
                .collect();
            code(
                format!(
                    "module {}",
                    path.trim_end_matches(".nst").replace('/', "::")
                ),
                &doc.join(" "),
            )
        }
        Def::Builtin(name) => {
            if let Some((_, d)) = BUILTIN_FNS.iter().find(|(n, _)| *n == name) {
                code(format!("fn {name}"), d)
            } else {
                code(
                    name.to_string(),
                    "Built-in machine type. Arithmetic wraps; convert explicitly between widths.",
                )
            }
        }
    }
}

fn doc_for(a: &Analysis, def: Def) -> String {
    let Some(span) = a.defs.get(&def) else {
        return String::new();
    };
    let m = &a.modules[a
        .modules
        .iter()
        .position(|m| m.file == span.file)
        .unwrap_or(0)];
    let text = &a.sources.get(m.file).text;
    // `///` lines right above the item's line.
    let line_start = text[..span.start as usize].rfind('\n').map_or(0, |i| i + 1);
    let mut lines = Vec::new();
    let mut end = line_start;
    for c in m.comments.iter().rev() {
        if c.span.end as usize > end {
            continue;
        }
        let between = &text[c.span.end as usize..end];
        if between.matches('\n').count() > 1
            || !between.trim().is_empty() && !between.trim().starts_with('@')
        {
            break;
        }
        match c.text.strip_prefix("///") {
            Some(t) => lines.push(t.trim().to_owned()),
            None => break,
        }
        end = c.span.start as usize;
    }
    lines.reverse();
    lines.join("\n")
}

fn completion_item(label: &str, kind: f64, detail: String) -> Json {
    obj([
        ("label", label.into()),
        ("kind", Json::Number(kind)),
        ("detail", detail.into()),
    ])
}

fn def_kind(def: &Def) -> f64 {
    match def {
        Def::Fn(_) => 3.0,
        Def::Global(_) => 6.0,
        Def::Struct(_) => 22.0,
        Def::Enum(_) => 13.0,
        Def::Variant(..) => 20.0,
        Def::Range(_) | Def::Alias(_) => 7.0,
        Def::Module(_) => 9.0,
        Def::Local(..) => 6.0,
        Def::Field(..) => 5.0,
        Def::Builtin(_) => 14.0,
    }
}

fn short(a: &Analysis, def: Def) -> String {
    let d = describe(a, def);
    d.lines().nth(1).unwrap_or("").to_owned()
}

fn completion(a: &Analysis, file: crate::source::FileId, text: &str, offset: usize) -> Json {
    let before = &text[..offset.min(text.len())];
    let word_start = before
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(0, |i| i + 1);
    let prefix_text = &before[..word_start];
    let module = a.modules.iter().position(|m| m.file == file).unwrap_or(0);
    let mut items = Vec::new();
    // `path::` completions.
    if prefix_text.ends_with("::") {
        let path_start = prefix_text[..prefix_text.len() - 2]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
            .map_or(0, |i| i + 1);
        let path = &prefix_text[path_start..prefix_text.len() - 2];
        let segs: Vec<&str> = path.split("::").collect();
        // Resolve the path's first segment in module scope.
        let mut target: Option<Def> = a.modules[module].names.get(segs[0]).copied();
        if target.is_none() {
            let file = format!("{}.nst", segs.join("/"));
            target = a
                .modules
                .iter()
                .position(|m| m.path == file)
                .map(|i| Def::Module(i as u32));
            if target.is_some() {
                return module_items(a, target.unwrap(), &mut items);
            }
            if segs == ["std"] {
                for m in &a.modules {
                    if let Some(rest) = m.path.strip_prefix("std/") {
                        items.push(completion_item(
                            rest.trim_end_matches(".nst"),
                            9.0,
                            "module".into(),
                        ));
                    }
                }
                for (p, _) in crate::stdlib::FILES {
                    let n = p.trim_start_matches("std/").trim_end_matches(".nst");
                    items.push(completion_item(n, 9.0, "standard module".into()));
                }
                items.push(completion_item("board", 9.0, "standard module".into()));
                return Json::Array(items);
            }
        }
        for s in &segs[1..] {
            target = match target {
                Some(Def::Module(m)) => a.modules[m as usize].names.get(*s).copied(),
                _ => None,
            };
        }
        if let Some(t) = target {
            return module_items(a, t, &mut items);
        }
        return Json::Array(items);
    }
    // Field completion after `name.`.
    if prefix_text.ends_with('.') {
        let base_end = prefix_text.len() - 1;
        let base_start = prefix_text[..base_end]
            .rfind(|c: char| {
                !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '[' || c == ']')
            })
            .map_or(0, |i| i + 1);
        let base = &prefix_text[base_start..base_end];
        let name = base.split(['[', '.']).next().unwrap_or("");
        let ty = find_type(a, file, module, name, offset);
        if let Some(mut ty) = ty {
            // Walk `.field` and `[..]` steps.
            let rest = &base[name.len()..];
            for part in rest
                .split('.')
                .skip(if rest.starts_with('.') { 1 } else { 0 })
            {
                let idx = part.matches('[').count();
                let fname = part.split('[').next().unwrap_or("");
                if !fname.is_empty() {
                    if let Type::Struct(s) = &ty {
                        if let Some((_, ft)) = a.program.types.structs[s.0 as usize]
                            .fields
                            .iter()
                            .find(|(n, _)| n == fname)
                        {
                            ty = ft.clone();
                        }
                    }
                }
                for _ in 0..idx {
                    if let Type::Array(e, _) = &ty {
                        ty = (**e).clone();
                    }
                }
            }
            for _ in 0..base.matches('[').count().min(1) {
                if let Type::Array(e, _) = &ty {
                    ty = (**e).clone();
                }
            }
            if let Type::Struct(s) = ty {
                for (n, ft) in &a.program.types.structs[s.0 as usize].fields {
                    items.push(completion_item(n, 5.0, a.program.types.name(ft)));
                }
            }
        }
        return Json::Array(items);
    }
    if prefix_text.ends_with('@') {
        for at in [
            "inline",
            "noinline",
            "nmi",
            "irq",
            "reset",
            "zp",
            "ram",
            "at",
            "io",
            "bank",
            "shared",
            "recursive",
            "budget",
            "export",
            "cold",
        ] {
            items.push(completion_item(at, 14.0, "attribute".into()));
        }
        return Json::Array(items);
    }
    // Locals of the enclosing function.
    for (i, f) in a.program.functions.iter().enumerate() {
        let _ = i;
        if f.span.file != file {
            continue;
        }
        for (k, l) in f.locals.iter().enumerate() {
            if l.span.file == file && (l.span.start as usize) < offset {
                let _ = k;
                items.push(completion_item(&l.name, 6.0, a.program.types.name(&l.ty)));
            }
        }
    }
    for (n, d) in &a.modules[module].names {
        items.push(completion_item(n, def_kind(d), short(a, *d)));
    }
    for g in &a.modules[module].globs {
        for (n, d) in &a.modules[*g as usize].names {
            items.push(completion_item(n, def_kind(d), short(a, *d)));
        }
    }
    for k in KEYWORDS {
        items.push(completion_item(k, 14.0, "keyword".into()));
    }
    for (b, d) in BUILTIN_FNS {
        items.push(completion_item(b, 3.0, d.to_string()));
    }
    for t in crate::check::BUILTIN_TYPES {
        items.push(completion_item(t, 7.0, "type".into()));
    }
    // Deduplicate by label.
    let mut seen = BTreeSet::new();
    items.retain(|i| seen.insert(i.get("label").str().to_owned()));
    Json::Array(items)
}

fn module_items(a: &Analysis, target: Def, items: &mut Vec<Json>) -> Json {
    match target {
        Def::Module(m) => {
            for (n, d) in &a.modules[m as usize].names {
                let public = a.defs.contains_key(d);
                if public {
                    items.push(completion_item(n, def_kind(d), short(a, *d)));
                }
            }
        }
        Def::Enum(e) => {
            for (i, (n, _)) in a.program.types.enums[e.0 as usize]
                .variants
                .iter()
                .enumerate()
            {
                items.push(completion_item(
                    n,
                    20.0,
                    short(a, Def::Variant(e, i as u32)),
                ));
            }
        }
        _ => {}
    }
    Json::Array(std::mem::take(items))
}

fn find_type(
    a: &Analysis,
    file: crate::source::FileId,
    module: usize,
    name: &str,
    offset: usize,
) -> Option<Type> {
    for f in &a.program.functions {
        if f.span.file != file {
            continue;
        }
        if let Some(l) = f
            .locals
            .iter()
            .rev()
            .find(|l| l.name == name && (l.span.start as usize) < offset)
        {
            return Some(l.ty.clone());
        }
    }
    match a.modules[module].names.get(name) {
        Some(Def::Global(g)) => Some(a.program.global(*g).ty.clone()),
        _ => None,
    }
}

fn signature(a: &Analysis, file: crate::source::FileId, text: &str, offset: usize) -> Json {
    // Find the innermost unclosed `(`.
    let before = &text[..offset.min(text.len())];
    let mut depth = 0i32;
    let mut commas = 0usize;
    let mut open = None;
    for (i, c) in before.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' if depth == 0 => {
                open = Some(i);
                break;
            }
            '(' => depth -= 1,
            ',' if depth == 0 => commas += 1,
            ';' | '{' | '}' if depth == 0 => break,
            _ => {}
        }
    }
    let Some(open) = open else { return Json::Null };
    let name_end = open;
    let name_start = before[..name_end]
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
        .map_or(0, |i| i + 1);
    let callee = &before[name_start..name_end];
    let last = callee.rsplit("::").next().unwrap_or("");
    // Resolve through refs at that position.
    let def = a
        .refs
        .iter()
        .filter(|(s, _)| {
            s.file == file && s.start as usize >= name_start && s.end as usize <= name_end
        })
        .map(|(_, d)| *d)
        .last()
        .or_else(|| {
            let module = a.modules.iter().position(|m| m.file == file).unwrap_or(0);
            a.modules[module].names.get(last).copied()
        });
    let Some(Def::Fn(f)) = def else {
        return Json::Null;
    };
    let func = a.program.function(f);
    let params: Vec<String> = func
        .params
        .iter()
        .map(|l| {
            let d = &func.locals[l.0 as usize];
            format!("{}: {}", d.name, a.program.types.name(&d.ty))
        })
        .collect();
    let label = format!("{}({})", func.name, params.join(", "));
    obj([
        (
            "signatures",
            Json::Array(vec![obj([
                ("label", label.into()),
                ("documentation", doc_for(a, Def::Fn(f)).into()),
                (
                    "parameters",
                    Json::Array(
                        params
                            .iter()
                            .map(|p| obj([("label", p.clone().into())]))
                            .collect(),
                    ),
                ),
            ])]),
        ),
        ("activeSignature", Json::Number(0.0)),
        ("activeParameter", Json::Number(commas as f64)),
    ])
}

fn semantic_tokens(a: &Analysis, file: crate::source::FileId, ws: &Workspace) -> Json {
    let _ = ws;
    let m = a.modules.iter().find(|m| m.file == file);
    let Some(m) = m else {
        return obj([("data", Json::Array(vec![]))]);
    };
    let src = a.sources.get(file);
    let mut refs: BTreeMap<u32, Def> = BTreeMap::new();
    for (s, d) in &a.refs {
        if s.file == file {
            refs.insert(s.start, *d);
        }
    }
    for (d, s) in &a.defs {
        if s.file == file {
            refs.insert(s.start, *d);
        }
    }
    let mut data = Vec::new();
    let (mut last_line, mut last_col) = (0usize, 0usize);
    let mut push = |start: usize, len: usize, kind: usize, modifiers: usize| {
        let (line, col) = src.position(start);
        let dl = line - last_line;
        let dc = if dl == 0 { col - last_col } else { col };
        data.extend([dl, dc, len, kind, modifiers].map(|n| Json::Number(n as f64)));
        last_line = line;
        last_col = col;
    };
    let mut prev_at = false;
    for t in &m.tokens {
        let start = t.span.start as usize;
        let len = (t.span.end - t.span.start) as usize;
        match &t.tok {
            Tok::Ident(w) => {
                if prev_at {
                    push(start, len, 11, 0);
                } else if KEYWORDS.contains(&w.as_str()) {
                    push(start, len, 5, 0);
                } else if let Some(d) = refs.get(&t.span.start) {
                    let (kind, modifiers) = match d {
                        Def::Module(_) => (0, 0),
                        Def::Struct(_) | Def::Enum(_) | Def::Range(_) | Def::Alias(_) => (1, 0),
                        Def::Fn(_) => (2, 0),
                        Def::Global(g) => (
                            3,
                            if a.program.global(*g).is_const() {
                                2
                            } else {
                                0
                            },
                        ),
                        Def::Local(f, l) => {
                            let func = a.program.function(*f);
                            if func.params.contains(l) {
                                (4, 0)
                            } else {
                                (3, 0)
                            }
                        }
                        Def::Variant(..) => (9, 0),
                        Def::Field(..) => (10, 0),
                        Def::Builtin(n) => {
                            if crate::check::BUILTIN_TYPES.contains(n) {
                                (1, 4)
                            } else {
                                (2, 4)
                            }
                        }
                    };
                    push(start, len, kind, modifiers);
                } else if crate::check::BUILTIN_TYPES.contains(&w.as_str()) {
                    push(start, len, 1, 4);
                }
            }
            Tok::Int(_) | Tok::Char(_) => push(start, len, 6, 0),
            Tok::Str(_) => push(start, len, 7, 0),
            _ => {}
        }
        prev_at = matches!(t.tok, Tok::Punct("@"));
    }
    obj([("data", Json::Array(data))])
}

fn document_symbols(a: &Analysis, file: crate::source::FileId) -> Json {
    let mut out = Vec::new();
    for (d, s) in &a.defs {
        if s.file != file {
            continue;
        }
        let kind = match d {
            Def::Fn(_) => 12.0,
            Def::Global(g) => {
                if a.program.global(*g).is_const() {
                    14.0
                } else {
                    13.0
                }
            }
            Def::Struct(_) => 23.0,
            Def::Enum(_) => 10.0,
            Def::Range(_) | Def::Alias(_) => 26.0,
            _ => continue,
        };
        let name = a.sources.text(*s).to_owned();
        out.push(obj([
            ("name", name.into()),
            ("kind", Json::Number(kind)),
            ("range", span_range(a, *s)),
            ("selectionRange", span_range(a, *s)),
        ]));
    }
    Json::Array(out)
}

/// Serve LSP over stdin/stdout with `Content-Length` framing, reading
/// project files from disk.
pub fn serve(
    input: &mut dyn std::io::BufRead,
    output: &mut dyn std::io::Write,
    loader: Option<Loader>,
) -> Result<(), String> {
    let mut server = LanguageServer::new();
    server.loader = loader;
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            if input.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Ok(());
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                length = v.trim().parse::<usize>().ok();
            }
        }
        let Some(n) = length else { continue };
        let mut body = vec![0; n];
        input.read_exact(&mut body).map_err(|e| e.to_string())?;
        let Ok(text) = std::str::from_utf8(&body) else {
            continue;
        };
        let Ok(request) = Json::parse(text) else {
            continue;
        };
        for reply in server.handle(request) {
            let s = reply.stringify();
            write!(output, "Content-Length: {}\r\n\r\n{s}", s.len()).map_err(|e| e.to_string())?;
        }
        output.flush().map_err(|e| e.to_string())?;
        if server.exit {
            return Ok(());
        }
    }
}

/// The native server: stdio with a filesystem loader for `file://` URIs.
pub fn serve_stdio() -> Result<(), String> {
    let loader: Loader = Box::new(|uri: &str| {
        let mut out = Vec::new();
        let Some(path) = uri.strip_prefix("file://") else {
            return out;
        };
        let path = percent_decode(path);
        let p = std::path::Path::new(&path);
        if path.ends_with(".nst") {
            if let Ok(b) = std::fs::read(p) {
                out.push((uri.to_owned(), b));
            }
            return out;
        }
        fn walk(
            dir: &std::path::Path,
            base_uri: &str,
            base: &std::path::Path,
            out: &mut Vec<(String, Vec<u8>)>,
            depth: usize,
        ) {
            if depth > 6 {
                return;
            }
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                if p.is_dir() {
                    walk(&p, base_uri, base, out, depth + 1);
                } else if std::fs::metadata(&p)
                    .map(|m| m.len() <= 1 << 20)
                    .unwrap_or(false)
                {
                    if let (Ok(rel), Ok(b)) = (p.strip_prefix(base), std::fs::read(&p)) {
                        out.push((
                            format!("{base_uri}{}", rel.to_string_lossy().replace('\\', "/")),
                            b,
                        ));
                    }
                }
            }
        }
        walk(p, uri, p, &mut out, 0);
        out
    });
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serve(&mut input, &mut output, Some(loader))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
