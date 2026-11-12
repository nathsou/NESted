//! Module loading, name resolution and item-level semantic analysis.
//!
//! Everything is resolved on demand: an array length may need a constant,
//! whose initializer may call a function evaluated at compile time, whose
//! body needs further types. Cycles are reported instead of looping.
use crate::ast::{self, Item, ItemKind, TypeExpr};
use crate::diag::{Diagnostic, Diagnostics};
use crate::hir::*;
use crate::lexer::{Comment, Token};
use crate::source::{FileId, SourceMap, Span};
use crate::types::*;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// What a name refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Def {
    Global(GlobalId),
    Fn(FnId),
    Struct(StructId),
    Enum(EnumId),
    Variant(EnumId, u32),
    Range(RangeId),
    Alias(u32),
    Module(u32),
    Local(FnId, LocalId),
    Field(StructId, u32),
    Builtin(&'static str),
}

pub struct ModuleInfo {
    pub path: String,
    pub file: FileId,
    pub ast: ast::Module,
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
    /// Items defined or imported here.
    pub names: BTreeMap<String, Def>,
    /// Glob imports (`use m::*`).
    pub globs: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum State {
    Pending,
    Busy,
    Done,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ItemRef {
    pub module: u32,
    pub item: u32,
}

/// Inputs supplied by the host. The compiler performs no I/O.
#[derive(Clone, Debug, Default)]
pub struct Project {
    /// Source files keyed by project-relative path (`main.nst`, `enemies/ai.nst`).
    pub sources: BTreeMap<String, String>,
    /// Binary assets keyed by project-relative path.
    pub assets: BTreeMap<String, Vec<u8>>,
    /// The root module's path.
    pub entry: String,
}

impl Project {
    pub fn single(source: &str) -> Self {
        let mut sources = BTreeMap::new();
        sources.insert("main.nst".to_owned(), source.to_owned());
        Self {
            sources,
            assets: BTreeMap::new(),
            entry: "main.nst".into(),
        }
    }
}

/// Result of analysis: a typed program plus editor information.
pub struct Analysis {
    pub sources: SourceMap,
    pub program: Program,
    pub diagnostics: Diagnostics,
    pub modules: Vec<ModuleInfo>,
    /// Every resolved name use.
    pub refs: Vec<(Span, Def)>,
    /// Definition sites.
    pub defs: BTreeMap<Def, Span>,
    /// Inferred types of `let` bindings without annotations: (name span, type).
    pub hints: Vec<(Span, Type)>,
    pub aliases: Vec<(String, Type)>,
}

pub(crate) struct Checker<'p> {
    pub project: &'p Project,
    pub sources: SourceMap,
    pub diags: Diagnostics,
    pub modules: Vec<ModuleInfo>,
    pub types: TypeTable,
    pub globals: Vec<Global>,
    pub functions: Vec<Option<Rc<Function>>>,
    pub body_state: Vec<State>,
    pub fn_sigs: Vec<Option<(Vec<(String, Type, bool)>, Type)>>,
    pub fn_meta: Vec<(String, String, Span, usize)>,
    pub item_of: BTreeMap<Def, ItemRef>,
    pub state: BTreeMap<Def, State>,
    pub alias_types: Vec<Option<Type>>,
    pub alias_names: Vec<String>,
    pub refs: Vec<(Span, Def)>,
    pub defs: BTreeMap<Def, Span>,
    pub hints: Vec<(Span, Type)>,
    pub cartridge: Cartridge,
    /// Hidden constant globals for string literals, deduplicated.
    pub strings: BTreeMap<Vec<u8>, GlobalId>,
    pub root: u32,
}

pub fn std_source(path: &str) -> Option<&'static str> {
    crate::stdlib::FILES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, s)| *s)
}

pub const BUILTIN_TYPES: &[&str] = &["u8", "i8", "u16", "i16", "bool"];
pub const BUILTIN_FNS: &[(&str, &str)] = &[
    ("lo", "lo(x: u16) -> u8: low byte of a word."),
    ("hi", "hi(x: u16) -> u8: high byte of a word."),
    ("len", "len(a) -> integer: length of an array (constant) or slice (u16)."),
    ("peek", "peek(address: u16) -> u8: volatile read of any CPU address."),
    ("poke", "poke(address: u16, value: u8): volatile write to any CPU address."),
    ("asset", "asset(\"path\") or asset(\"path\", start..end): bytes of a project asset, at compile time."),
    ("defined", "defined(path) -> bool: whether a name resolves, at compile time."),
    ("addr", "addr(x) -> u16: CPU address of a global or of a slice's first element."),
];

pub fn analyze(project: &Project) -> Analysis {
    let mut c = Checker {
        project,
        sources: SourceMap::default(),
        diags: Diagnostics::default(),
        modules: Vec::new(),
        types: TypeTable::default(),
        globals: Vec::new(),
        functions: Vec::new(),
        body_state: Vec::new(),
        fn_sigs: Vec::new(),
        fn_meta: Vec::new(),
        item_of: BTreeMap::new(),
        state: BTreeMap::new(),
        alias_types: Vec::new(),
        alias_names: Vec::new(),
        refs: Vec::new(),
        defs: BTreeMap::new(),
        hints: Vec::new(),
        cartridge: Cartridge::default(),
        strings: BTreeMap::new(),
        root: 0,
    };
    c.load_all();
    c.collect();
    c.resolve_imports();
    c.check_all();
    c.finish()
}

fn module_path_of_file(path: &str) -> String {
    path.trim_end_matches(".nst").replace('/', "::")
}

impl<'p> Checker<'p> {
    // ------------------------------------------------------------- loading

    fn load(&mut self, path: &str, span: Option<Span>) -> Option<u32> {
        if let Some(i) = self.modules.iter().position(|m| m.path == path) {
            return Some(i as u32);
        }
        let text = if path == "std/board.nst" {
            Some(self.board_source())
        } else if let Some(rest) = path.strip_prefix("std/") {
            std_source(&format!("std/{rest}")).map(str::to_owned)
        } else {
            self.project.sources.get(path).cloned()
        };
        let Some(text) = text else {
            if let Some(span) = span {
                self.diags
                    .error(span, format!("Cannot find module file '{path}'"));
            } else {
                self.diags
                    .error(Span::default(), format!("Cannot find source file '{path}'"));
            }
            return None;
        };
        let file = self.sources.add(path, &text);
        let (ast, tokens, comments, d) = crate::parser::parse(file, &text);
        self.diags.extend(d);
        let id = self.modules.len() as u32;
        let uses: Vec<ast::Path> = ast
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::Use(u) => Some(u.path.clone()),
                _ => None,
            })
            .collect();
        self.modules.push(ModuleInfo {
            path: path.to_owned(),
            file,
            ast,
            tokens,
            comments,
            names: BTreeMap::new(),
            globs: Vec::new(),
        });
        for u in uses {
            if let Some(file) = self.module_file(&u, false) {
                self.load(&file, Some(u.span));
            }
        }
        Some(id)
    }

    /// `std::board`: constants describing the declared cartridge.
    fn board_source(&self) -> String {
        let mut mapper = "nrom".to_owned();
        let mut mirroring = "vertical".to_owned();
        let mut chr_ram = false;
        let mut queue: i64 = 160;
        let mut sprites = true;
        if let Some(root) = self.modules.get(self.root as usize) {
            for item in &root.ast.items {
                if let ItemKind::Cartridge(fields) = &item.kind {
                    for (name, value) in fields {
                        if let (ast::ExprKind::Int(n), "queue") = (&value.kind, name.name.as_str())
                        {
                            queue = (*n).clamp(16, 250);
                        }
                        if let (ast::ExprKind::Bool(b), "sprites") =
                            (&value.kind, name.name.as_str())
                        {
                            sprites = *b;
                        }
                    }
                }
            }
            for item in &root.ast.items {
                if let ItemKind::Cartridge(fields) = &item.kind {
                    for (name, value) in fields {
                        if let ast::ExprKind::Path(p) = &value.kind {
                            let w = p.last().name.clone();
                            match name.name.as_str() {
                                "mapper" => mapper = w,
                                "mirroring" => mirroring = w,
                                "chr" if w == "ram" => chr_ram = true,
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        let board = crate::link::Board::from_name(&mapper).unwrap_or(crate::link::Board::Nrom);
        let chr_ram = chr_ram || !board.chr_rom();
        let mut out = String::from("//! Constants describing this program's cartridge board.\n");
        out.push_str(&format!(
            "/// iNES mapper number.\npub const MAPPER: u16 = {};\n",
            board.ines()
        ));
        for n in crate::link::Board::NAMES {
            let b = crate::link::Board::from_name(n).unwrap();
            out.push_str(&format!(
                "pub const {}: bool = {};\n",
                n.to_uppercase(),
                b == board
            ));
        }
        out.push_str(&format!("/// Pattern tables are RAM (uploaded by the program).\npub const CHR_RAM: bool = {chr_ram};\n"));
        out.push_str(&format!(
            "pub const HORIZONTAL: bool = {};\n",
            mirroring == "horizontal"
        ));
        out.push_str(&format!(
            "pub const PRG_BANKING: bool = {};\n",
            board.banking() != crate::codegen::Banking::None
        ));
        out.push_str(&format!("/// Bytes of background updates per frame (`queue` in the cartridge block).\npub const QUEUE: u8 = {queue};\n"));
        out.push_str(&format!("/// Sprites are drawn (`sprites = false` frees the shadow sprite page).\npub const SPRITES: bool = {sprites};\n"));
        out
    }

    /// File for a module path; `use a::b::item` may name an item in `a/b.nst`.
    fn module_file(&self, path: &ast::Path, exact: bool) -> Option<String> {
        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        if segs[0] == "main" {
            return None;
        }
        let candidates = if exact { 1 } else { 2 };
        for drop in 0..candidates.min(segs.len()) {
            let n = segs.len() - drop;
            if n == 0 {
                break;
            }
            let file = format!("{}.nst", segs[..n].join("/"));
            let exists = if file.starts_with("std/") {
                std_source(&file).is_some() || file == "std/board.nst"
            } else {
                self.project.sources.contains_key(&file)
            };
            if exists || (drop == 0 && candidates == 1) {
                return Some(file);
            }
        }
        Some(format!("{}.nst", segs.join("/")))
    }

    fn load_all(&mut self) {
        let entry = self.project.entry.clone();
        self.load(&entry, None);
        // The standard frame runtime is linked unless a custom @reset exists.
        let custom = self
            .modules
            .iter()
            .any(|m| m.ast.items.iter().any(|i| i.attr("reset").is_some()));
        if !custom && !self.modules.is_empty() {
            self.load("std/runtime.nst", None);
        }
    }

    // ---------------------------------------------------------- collection

    fn collect(&mut self) {
        for m in 0..self.modules.len() {
            let items: Vec<(usize, Item)> = self.modules[m]
                .ast
                .items
                .iter()
                .cloned()
                .enumerate()
                .collect();
            for (i, item) in items {
                let r = ItemRef {
                    module: m as u32,
                    item: i as u32,
                };
                let def = match &item.kind {
                    ItemKind::Use(_) => continue,
                    ItemKind::Cartridge(_) => {
                        if m as u32 != self.root {
                            self.diags
                                .error(item.span, "Only the root module may declare a cartridge");
                        }
                        continue;
                    }
                    ItemKind::Const { name, .. } | ItemKind::Var { name, .. } => {
                        let id = GlobalId(self.globals.len() as u32);
                        let kind = match &item.kind {
                            ItemKind::Const { .. } => GlobalKind::Const,
                            _ => GlobalKind::Var,
                        };
                        self.globals.push(Global {
                            name: name.name.clone(),
                            path: self.qualify(m, &name.name),
                            ty: Type::Error,
                            kind,
                            value: Value::Void,
                            placement: Placement::Auto,
                            bank: None,
                            shared: false,
                            span: name.span,
                            module: m,
                            doc: self.doc(m, &item),
                        });
                        Def::Global(id)
                    }
                    ItemKind::Fn(f) => {
                        let id = FnId(self.functions.len() as u32);
                        self.functions.push(None);
                        self.body_state.push(State::Pending);
                        self.fn_sigs.push(None);
                        self.fn_meta.push((
                            f.name.name.clone(),
                            self.qualify(m, &f.name.name),
                            f.name.span,
                            m,
                        ));
                        Def::Fn(id)
                    }
                    ItemKind::Struct { name, .. } => {
                        let id = StructId(self.types.structs.len() as u32);
                        self.types.structs.push(StructDef {
                            name: name.name.clone(),
                            fields: Vec::new(),
                            span: name.span,
                        });
                        Def::Struct(id)
                    }
                    ItemKind::Enum { name, .. } => {
                        let id = EnumId(self.types.enums.len() as u32);
                        self.types.enums.push(EnumDef {
                            name: name.name.clone(),
                            repr: Type::U8,
                            variants: Vec::new(),
                            span: name.span,
                        });
                        Def::Enum(id)
                    }
                    ItemKind::Type { name, ty } => {
                        if let TypeExpr::Range(..) = ty {
                            let id = RangeId(self.types.ranges.len() as u32);
                            self.types.ranges.push(RangeDef {
                                name: name.name.clone(),
                                base: Type::U8,
                                lo: 0,
                                hi: 0,
                                span: name.span,
                            });
                            Def::Range(id)
                        } else {
                            let id = self.alias_types.len() as u32;
                            self.alias_types.push(None);
                            self.alias_names.push(name.name.clone());
                            Def::Alias(id)
                        }
                    }
                };
                let name = item.name().unwrap().clone();
                self.item_of.insert(def, r);
                self.state.insert(def, State::Pending);
                self.defs.insert(def, name.span);
                if let Some(old) = self.modules[m].names.insert(name.name.clone(), def) {
                    let prev = self.defs.get(&old).copied().unwrap_or_default();
                    self.diags.push(
                        Diagnostic::error(name.span, format!("'{}' is defined twice", name.name))
                            .note(prev, "first definition"),
                    );
                }
                if let Def::Enum(e) = def {
                    if let ItemKind::Enum { variants, .. } = &item.kind {
                        for (i, (v, _)) in variants.iter().enumerate() {
                            self.defs.insert(Def::Variant(e, i as u32), v.span);
                        }
                    }
                }
                if let Def::Struct(s) = def {
                    if let ItemKind::Struct { fields, .. } = &item.kind {
                        for (i, (f, _)) in fields.iter().enumerate() {
                            self.defs.insert(Def::Field(s, i as u32), f.span);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn qualify(&self, module: usize, name: &str) -> String {
        let p = module_path_of_file(&self.modules[module].path);
        if module as u32 == self.root {
            name.to_owned()
        } else {
            format!("{p}::{name}")
        }
    }

    /// `///` comments immediately preceding an item.
    fn doc(&self, module: usize, item: &Item) -> String {
        let m = &self.modules[module];
        let text = &self.sources.get(m.file).text;
        let mut lines = Vec::new();
        let mut end = item.span.start as usize;
        for c in m.comments.iter().rev() {
            if c.span.end as usize > end {
                continue;
            }
            let between = &text[c.span.end as usize..end];
            if !between.trim().is_empty() || between.matches('\n').count() > 1 {
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

    fn resolve_imports(&mut self) {
        for m in 0..self.modules.len() {
            let uses: Vec<ast::UseTree> = self.modules[m]
                .ast
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    ItemKind::Use(u) => Some(u.clone()),
                    _ => None,
                })
                .collect();
            for u in uses {
                self.import(m, &u);
            }
        }
    }

    fn module_by_path(&self, segs: &[ast::Ident]) -> Option<u32> {
        if segs.len() == 1 && segs[0].name == "main" {
            return Some(self.root);
        }
        let file = format!(
            "{}.nst",
            segs.iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join("/")
        );
        self.modules
            .iter()
            .position(|m| m.path == file)
            .map(|i| i as u32)
    }

    fn import(&mut self, m: usize, u: &ast::UseTree) {
        let segs = &u.path.segments;
        let bind = |c: &mut Self, name: &ast::Ident, def: Def| {
            if let Some(old) = c.modules[m].names.insert(name.name.clone(), def) {
                if old != def {
                    c.diags.error(
                        name.span,
                        format!("'{}' is already defined here", name.name),
                    );
                }
            }
        };
        if let Some(target) = self.module_by_path(segs) {
            for (i, s) in segs.iter().enumerate() {
                if let Some(mm) = self.module_by_path(&segs[..=i]) {
                    self.refs.push((s.span, Def::Module(mm)));
                }
            }
            if u.glob {
                self.modules[m].globs.push(target);
            } else if u.names.is_empty() {
                let name = u
                    .alias
                    .clone()
                    .unwrap_or_else(|| segs.last().unwrap().clone());
                bind(self, &name, Def::Module(target));
            } else {
                for (n, alias) in &u.names {
                    match self.modules[target as usize].names.get(&n.name).copied() {
                        Some(def) => {
                            self.check_visible(def, m, n.span);
                            self.refs.push((n.span, def));
                            bind(self, alias.as_ref().unwrap_or(n), def);
                        }
                        None => self.diags.error(
                            n.span,
                            format!("Module '{}' has no item '{}'", u.path.text(), n.name),
                        ),
                    }
                }
            }
            return;
        }
        // `use module::item`.
        if segs.len() >= 2 {
            if let Some(target) = self.module_by_path(&segs[..segs.len() - 1]) {
                let n = segs.last().unwrap();
                match self.modules[target as usize].names.get(&n.name).copied() {
                    Some(def) => {
                        self.check_visible(def, m, n.span);
                        self.refs.push((n.span, def));
                        bind(self, u.alias.as_ref().unwrap_or(n), def);
                    }
                    None => self.diags.error(
                        n.span,
                        format!(
                            "Cannot find '{}' in '{}'",
                            n.name,
                            segs[..segs.len() - 1]
                                .iter()
                                .map(|s| s.name.as_str())
                                .collect::<Vec<_>>()
                                .join("::")
                        ),
                    ),
                }
            }
        }
        // Loading already reported missing files.
    }

    pub(crate) fn check_visible(&mut self, def: Def, from: usize, span: Span) {
        let Some(r) = self.item_of.get(&def).copied() else {
            return;
        };
        if r.module as usize == from {
            return;
        }
        // The standard library may call the program's hooks (`main::init`).
        if r.module == self.root && self.modules[from].path.starts_with("std/") {
            return;
        }
        let item = &self.modules[r.module as usize].ast.items[r.item as usize];
        if !item.public {
            let name = item.name().map(|n| n.name.clone()).unwrap_or_default();
            self.diags.error(
                span,
                format!("'{name}' is private to its module; mark it 'pub'"),
            );
        }
    }

    // ---------------------------------------------------------- resolution

    /// Look a name up in module scope (definitions, imports, globs, prelude).
    pub(crate) fn lookup(&self, m: usize, name: &str) -> Option<Def> {
        if let Some(d) = self.modules[m].names.get(name) {
            return Some(*d);
        }
        for g in &self.modules[m].globs {
            if let Some(d) = self.modules[*g as usize].names.get(name) {
                return Some(*d);
            }
        }
        if name == "main" {
            return Some(Def::Module(self.root));
        }
        if let Some(t) = BUILTIN_TYPES.iter().find(|t| **t == name) {
            return Some(Def::Builtin(t));
        }
        if let Some((f, _)) = BUILTIN_FNS.iter().find(|(f, _)| *f == name) {
            return Some(Def::Builtin(f));
        }
        if name == "std" {
            return None;
        }
        None
    }

    /// Resolve a module-scope path (no locals).
    pub(crate) fn resolve_path(&mut self, m: usize, path: &ast::Path, report: bool) -> Option<Def> {
        let segs = &path.segments;
        let mut def = self.lookup(m, &segs[0].name);
        let mut start = 1;
        if def.is_none() {
            // Fully-qualified module paths, such as `std::video::text`.
            for n in (1..=segs.len()).rev() {
                if let Some(mm) = self.module_by_path(&segs[..n]) {
                    def = Some(Def::Module(mm));
                    start = n;
                    break;
                }
            }
            if def.is_none() {
                if report {
                    self.diags.error(
                        segs[0].span,
                        format!("Cannot find '{}' in this scope", segs[0].name),
                    );
                }
                return None;
            }
        }
        self.refs.push((segs[start - 1].span, def.unwrap()));
        for seg in &segs[start..] {
            let next = match def.unwrap() {
                Def::Module(mm) => {
                    let d = self.modules[mm as usize].names.get(&seg.name).copied();
                    if let Some(d) = d {
                        self.check_visible(d, m, seg.span);
                    }
                    d
                }
                Def::Enum(e) => {
                    self.ensure(Def::Enum(e));
                    self.types.enums[e.0 as usize]
                        .variants
                        .iter()
                        .position(|v| v.0 == seg.name)
                        .map(|i| Def::Variant(e, i as u32))
                }
                _ => None,
            };
            match next {
                Some(d) => {
                    self.refs.push((seg.span, d));
                    def = Some(d);
                }
                None => {
                    if report {
                        self.diags.error(
                            seg.span,
                            format!("Cannot find '{}' in '{}'", seg.name, path.text()),
                        );
                    }
                    return None;
                }
            }
        }
        def
    }

    pub(crate) fn resolve_type(&mut self, m: usize, t: &TypeExpr) -> Type {
        match t {
            TypeExpr::Path(p) => match self.resolve_path(m, p, true) {
                Some(Def::Builtin(name)) => match name {
                    "u8" => Type::U8,
                    "i8" => Type::I8,
                    "u16" => Type::U16,
                    "i16" => Type::I16,
                    "bool" => Type::Bool,
                    _ => {
                        self.diags.error(p.span, format!("'{name}' is not a type"));
                        Type::Error
                    }
                },
                Some(d @ Def::Struct(s)) => {
                    self.ensure(d);
                    Type::Struct(s)
                }
                Some(d @ Def::Enum(e)) => {
                    self.ensure(d);
                    Type::Enum(e)
                }
                Some(d @ Def::Range(r)) => {
                    self.ensure(d);
                    Type::Range(r)
                }
                Some(d @ Def::Alias(a)) => {
                    self.ensure(d);
                    self.alias_types[a as usize].clone().unwrap_or(Type::Error)
                }
                Some(_) => {
                    self.diags
                        .error(p.span, format!("'{}' is not a type", p.text()));
                    Type::Error
                }
                None => Type::Error,
            },
            TypeExpr::Array(elem, len, span) => {
                let elem = self.resolve_type(m, elem);
                let Some(len) = len else {
                    self.diags
                        .error(*span, "An inferred array length '_' needs an initializer");
                    return Type::Error;
                };
                match self.const_int(m, len) {
                    Some(n) if (1..=65535).contains(&n) => Type::Array(Box::new(elem), n as u32),
                    Some(_) => {
                        self.diags
                            .error(len.span, "Array length must be between 1 and 65535");
                        Type::Error
                    }
                    None => Type::Error,
                }
            }
            TypeExpr::Slice(elem, span) => {
                let elem = self.resolve_type(m, elem);
                if self.types.size(&elem) != 1 || !self.types.is_scalar(&elem) {
                    self.diags
                        .error(*span, "Slices hold byte-sized scalar elements");
                    return Type::Error;
                }
                Type::Slice(Box::new(elem))
            }
            TypeExpr::Range(_, _, _, _, span) => {
                self.diags.error(
                    *span,
                    "Refined ranges are declared with 'type Name = u8 in lo..hi;'",
                );
                Type::Error
            }
        }
    }

    /// Type of an array with inferred length, from its initializer.
    pub(crate) fn resolve_decl_type(
        &mut self,
        m: usize,
        t: &TypeExpr,
        init: Option<&ast::Expr>,
    ) -> (Type, Option<Expr>) {
        if let TypeExpr::Array(elem, None, _) = t {
            if let Some(init) = init {
                let elem_ty = self.resolve_type(m, elem);
                let mut scope = crate::check_expr::Scope::constant(m);
                let e = self.expr(&mut scope, init, None);
                let n = match &e.ty {
                    Type::Array(_, n) => *n,
                    _ => {
                        self.diags.error(init.span, "Expected an array initializer");
                        return (Type::Error, None);
                    }
                };
                let ty = Type::Array(Box::new(elem_ty), n);
                let e = self.coerce(&mut scope, e, &ty);
                return (ty, Some(e));
            }
        }
        (self.resolve_type(m, t), None)
    }

    // ---------------------------------------------------------- item checks

    /// Resolve a definition's type-level information.
    pub(crate) fn ensure(&mut self, def: Def) {
        match self.state.get(&def).copied() {
            Some(State::Pending) => {}
            Some(State::Busy) => {
                if !matches!(def, Def::Fn(_)) {
                    let span = self.defs.get(&def).copied().unwrap_or_default();
                    self.diags.error(span, "This definition depends on itself");
                }
                return;
            }
            _ => return,
        }
        self.state.insert(def, State::Busy);
        let r = self.item_of[&def];
        let m = r.module as usize;
        let item = self.modules[m].ast.items[r.item as usize].clone();
        match (&item.kind, def) {
            (ItemKind::Struct { fields, .. }, Def::Struct(s)) => {
                let mut out = Vec::new();
                let mut seen = BTreeSet::new();
                for (name, t) in fields {
                    let ty = self.resolve_type(m, t);
                    if !seen.insert(name.name.clone()) {
                        self.diags
                            .error(name.span, format!("Field '{}' is repeated", name.name));
                    }
                    if matches!(ty, Type::Array(..) | Type::Slice(_)) {
                        self.diags.error(
                            t.span(),
                            "Struct fields are scalars or structs; keep arrays as separate globals",
                        );
                    }
                    out.push((name.name.clone(), ty));
                }
                self.types.structs[s.0 as usize].fields = out;
            }
            (ItemKind::Enum { repr, variants, .. }, Def::Enum(e)) => {
                let repr_ty = match repr {
                    Some(t) => self.resolve_type(m, t),
                    None => Type::U8,
                };
                if !repr_ty.is_machine_int() && !repr_ty.is_error() {
                    self.diags
                        .error(item.span, "Enum representation must be an integer type");
                }
                let mut next = 0i64;
                let mut out: Vec<(String, i64)> = Vec::new();
                let (lo, hi) = self.types.bounds(&repr_ty);
                for (name, value) in variants {
                    if let Some(v) = value {
                        if let Some(n) = self.const_int(m, v) {
                            next = n;
                        }
                    }
                    if next < lo || next > hi {
                        self.diags
                            .error(name.span, "Variant value does not fit the representation");
                    }
                    if out.iter().any(|(n, _)| *n == name.name) {
                        self.diags
                            .error(name.span, format!("Variant '{}' is repeated", name.name));
                    }
                    if out.iter().any(|(_, v)| *v == next) {
                        self.diags.error(name.span, "Two variants share a value");
                    }
                    out.push((name.name.clone(), next));
                    next += 1;
                }
                if out.is_empty() {
                    self.diags
                        .error(item.span, "An enum needs at least one variant");
                }
                let def = &mut self.types.enums[e.0 as usize];
                def.repr = repr_ty;
                def.variants = out;
            }
            (ItemKind::Type { ty, .. }, Def::Range(r)) => {
                if let TypeExpr::Range(base, lo, hi, inclusive, span) = ty {
                    let base = self.resolve_type(m, base);
                    if !base.is_machine_int() {
                        self.diags
                            .error(*span, "A range refines u8, i8, u16 or i16");
                    }
                    let lo = self.const_int(m, lo).unwrap_or(0);
                    let mut hi = self.const_int(m, hi).unwrap_or(0);
                    if !inclusive {
                        hi -= 1;
                    }
                    let (blo, bhi) = self.types.bounds(&base);
                    if lo > hi || lo < blo || hi > bhi {
                        self.diags.error(
                            *span,
                            format!(
                                "Range {lo}..={hi} is empty or does not fit {}",
                                self.types.name(&base)
                            ),
                        );
                    }
                    let def = &mut self.types.ranges[r.0 as usize];
                    def.base = base;
                    def.lo = lo;
                    def.hi = hi;
                }
            }
            (ItemKind::Type { ty, .. }, Def::Alias(a)) => {
                let t = self.resolve_type(m, ty);
                self.alias_types[a as usize] = Some(t);
            }
            (ItemKind::Const { ty, value, .. }, Def::Global(g)) => {
                self.check_const(m, g, ty.as_ref(), value, &item);
            }
            (ItemKind::Var { ty, value, .. }, Def::Global(g)) => {
                self.check_var(m, g, ty, value.as_ref(), &item);
            }
            (ItemKind::Fn(f), Def::Fn(id)) => {
                // Bodies are checked separately (see `body`), so mutually
                // recursive calls see each other's signatures.
                self.check_signature(m, id, f);
            }
            _ => {}
        }
        self.state.insert(def, State::Done);
    }

    fn global_attrs(&mut self, g: GlobalId, item: &Item, constant: bool) {
        for a in &item.attrs {
            let name = a.name.name.as_str();
            let arg = |c: &mut Self| -> Option<i64> {
                match a.args.first() {
                    Some(e) => c.const_int(c.globals[g.0 as usize].module, e),
                    None => {
                        c.diags.error(a.span, format!("@{name} needs an argument"));
                        None
                    }
                }
            };
            match name {
                "zp" if !constant => self.globals[g.0 as usize].placement = Placement::ZeroPage,
                "ram" if !constant => self.globals[g.0 as usize].placement = Placement::Ram,
                "at" if !constant => {
                    if let Some(n) = arg(self) {
                        self.globals[g.0 as usize].placement = Placement::At(n as u16);
                    }
                }
                "io" if !constant => {
                    if let Some(n) = arg(self) {
                        self.globals[g.0 as usize].kind = GlobalKind::Io(n as u16);
                    }
                }
                "shared" if !constant => self.globals[g.0 as usize].shared = true,
                "bank" if constant => {
                    if let Some(n) = arg(self) {
                        self.globals[g.0 as usize].bank = Some(n as u32);
                    }
                }
                _ => self.diags.error(
                    a.name.span,
                    format!(
                        "Unknown attribute @{name} for {}",
                        if constant { "a constant" } else { "a variable" }
                    ),
                ),
            }
        }
    }

    fn check_const(
        &mut self,
        m: usize,
        g: GlobalId,
        ty: Option<&TypeExpr>,
        value: &ast::Expr,
        item: &Item,
    ) {
        self.global_attrs(g, item, true);
        let mut scope = crate::check_expr::Scope::constant(m);
        let (t, e) = match ty {
            Some(t) => {
                let (decl, pre) = self.resolve_decl_type(m, t, Some(value));
                let e = match pre {
                    Some(e) => e,
                    None => {
                        let e = self.expr(&mut scope, value, Some(&decl));
                        self.coerce(&mut scope, e, &decl)
                    }
                };
                (decl, e)
            }
            None => {
                let e = self.expr(&mut scope, value, None);
                let t = self.default_type(&e);
                let e = self.coerce(&mut scope, e, &t);
                (t, e)
            }
        };
        self.globals[g.0 as usize].ty = t.clone();
        if t.is_error() || e.ty.is_error() {
            return;
        }
        if let Some(v) = self.eval(&e) {
            self.globals[g.0 as usize].value = v;
        }
    }

    fn check_var(
        &mut self,
        m: usize,
        g: GlobalId,
        ty: &TypeExpr,
        value: Option<&ast::Expr>,
        item: &Item,
    ) {
        self.global_attrs(g, item, false);
        let (t, pre) = self.resolve_decl_type(m, ty, value);
        self.globals[g.0 as usize].ty = t.clone();
        if matches!(t, Type::Slice(_)) {
            self.diags
                .error(ty.span(), "Slices cannot be stored in RAM in this version");
        }
        let io = matches!(self.globals[g.0 as usize].kind, GlobalKind::Io(_));
        match value {
            Some(v) if io => self
                .diags
                .error(v.span, "Hardware registers have no initializer"),
            Some(v) => {
                let mut scope = crate::check_expr::Scope::constant(m);
                let e = match pre {
                    Some(e) => e,
                    None => {
                        let e = self.expr(&mut scope, v, Some(&t));
                        self.coerce(&mut scope, e, &t)
                    }
                };
                if !e.ty.is_error() && !t.is_error() {
                    if let Some(v) = self.eval(&e) {
                        self.globals[g.0 as usize].value = v;
                    }
                }
            }
            None => {
                self.globals[g.0 as usize].value = self.zero(&t);
            }
        }
    }

    pub(crate) fn zero(&self, t: &Type) -> Value {
        match t {
            Type::Array(e, n) => Value::aggregate(vec![self.zero(e); *n as usize]),
            Type::Struct(s) => Value::aggregate(
                self.types.structs[s.0 as usize]
                    .fields
                    .iter()
                    .map(|(_, t)| self.zero(t))
                    .collect(),
            ),
            Type::Enum(e) => Value::Int(
                self.types.enums[e.0 as usize]
                    .variants
                    .first()
                    .map_or(0, |v| v.1),
            ),
            Type::Range(r) => Value::Int(self.types.ranges[r.0 as usize].lo),
            _ => Value::Int(0),
        }
    }

    fn check_signature(&mut self, m: usize, id: FnId, f: &ast::FnDecl) {
        let mut params = Vec::new();
        for p in &f.params {
            let t = self.resolve_type(m, &p.ty);
            if matches!(t, Type::Array(..)) {
                self.diags.error(
                    p.ty.span(),
                    "Arrays cannot be passed by value; pass an index or a slice",
                );
            }
            params.push((p.name.name.clone(), t, p.mutable));
        }
        let result = match &f.result {
            Some(t) => self.resolve_type(m, t),
            None => Type::Void,
        };
        self.fn_sigs[id.0 as usize] = Some((params, result));
    }

    pub(crate) fn fn_attrs(&mut self, m: usize, item: &Item) -> FnAttrs {
        let mut attrs = FnAttrs::default();
        for a in &item.attrs {
            let name = a.name.name.as_str();
            let int = |c: &mut Self| a.args.first().and_then(|e| c.const_int(m, e));
            match name {
                "inline" => attrs.inline = true,
                "noinline" => attrs.noinline = true,
                "nmi" => attrs.interrupt = Some(Context::Nmi),
                "irq" => attrs.interrupt = Some(Context::Irq),
                "reset" => attrs.reset = true,
                "export" => attrs.export = true,
                "cold" => attrs.cold = true,
                "recursive" => match int(self) {
                    Some(n) if (1..=64).contains(&n) => attrs.recursive = Some(n as u32),
                    _ => self
                        .diags
                        .error(a.span, "@recursive(N) needs a depth from 1 to 64"),
                },
                "budget" => match int(self) {
                    Some(n) if n > 0 => attrs.budget = Some(n as u32),
                    _ => self.diags.error(a.span, "@budget(N) needs a cycle count"),
                },
                "bank" => match int(self) {
                    Some(n) if n >= 0 => attrs.bank = Some(n as u32),
                    _ => self.diags.error(a.span, "@bank(N) needs a bank number"),
                },
                _ => self
                    .diags
                    .error(a.name.span, format!("Unknown function attribute @{name}")),
            }
        }
        if attrs.inline && attrs.noinline {
            self.diags
                .error(item.span, "@inline and @noinline conflict");
        }
        attrs
    }

    pub(crate) fn signature(&mut self, id: FnId) -> (Vec<(String, Type, bool)>, Type) {
        self.ensure(Def::Fn(id));
        self.fn_sigs[id.0 as usize]
            .clone()
            .unwrap_or((Vec::new(), Type::Error))
    }

    /// Check a function body (once) and return it.
    pub(crate) fn body(&mut self, id: FnId) -> Option<Rc<Function>> {
        if let Some(f) = &self.functions[id.0 as usize] {
            return Some(f.clone());
        }
        match self.body_state[id.0 as usize] {
            State::Busy => return None,
            State::Done => return self.functions[id.0 as usize].clone(),
            State::Pending => {}
        }
        self.body_state[id.0 as usize] = State::Busy;
        let (params, result) = self.signature(id);
        let r = self.item_of[&Def::Fn(id)];
        let m = r.module as usize;
        let item = self.modules[m].ast.items[r.item as usize].clone();
        let ItemKind::Fn(decl) = &item.kind else {
            return None;
        };
        let attrs = self.fn_attrs(m, &item);
        let f = self.check_body(m, id, decl, params, result, attrs, &item);
        let f = Rc::new(f);
        self.functions[id.0 as usize] = Some(f.clone());
        self.body_state[id.0 as usize] = State::Done;
        Some(f)
    }

    // ---------------------------------------------------------- cartridge

    fn check_cartridge(&mut self) {
        let root = self.root as usize;
        if root >= self.modules.len() {
            return;
        }
        let items: Vec<Item> = self.modules[root]
            .ast
            .items
            .iter()
            .filter(|i| matches!(i.kind, ItemKind::Cartridge(_)))
            .cloned()
            .collect();
        let mut cart = Cartridge {
            mapper: "nrom".into(),
            mirroring: "vertical".into(),
            ..Default::default()
        };
        if items.len() > 1 {
            self.diags
                .error(items[1].span, "Only one cartridge block is allowed");
        }
        let Some(item) = items.first() else {
            self.cartridge = cart;
            return;
        };
        cart.span = item.span;
        let ItemKind::Cartridge(fields) = &item.kind else {
            unreachable!()
        };
        let word = |e: &ast::Expr| match &e.kind {
            ast::ExprKind::Path(p) if p.is_single() => Some(p.segments[0].name.clone()),
            _ => None,
        };
        for (name, value) in fields {
            match name.name.as_str() {
                "title" | "name" => match &value.kind {
                    ast::ExprKind::Str(s) => cart.title = String::from_utf8_lossy(s).into(),
                    _ => self.diags.error(value.span, "Expected a string"),
                },
                "mapper" => match word(value) {
                    Some(w) if crate::link::Board::from_name(&w).is_some() => cart.mapper = w,
                    _ => self.diags.error(
                        value.span,
                        format!(
                            "Unknown mapper; expected one of {}",
                            crate::link::Board::NAMES.join(", ")
                        ),
                    ),
                },
                "mirroring" => match word(value).as_deref() {
                    Some(w @ ("horizontal" | "vertical" | "single" | "four")) => {
                        cart.mirroring = w.to_owned()
                    }
                    _ => self
                        .diags
                        .error(value.span, "Expected horizontal, vertical, single or four"),
                },
                "chr" => {
                    let mut paths = Vec::new();
                    match &value.kind {
                        ast::ExprKind::Str(s) => {
                            paths.push((String::from_utf8_lossy(s).to_string(), value.span))
                        }
                        ast::ExprKind::Array(items) => {
                            for i in items {
                                match &i.kind {
                                    ast::ExprKind::Str(s) => {
                                        paths.push((String::from_utf8_lossy(s).to_string(), i.span))
                                    }
                                    _ => self.diags.error(i.span, "Expected a CHR asset path"),
                                }
                            }
                        }
                        ast::ExprKind::Path(p) if p.is_single() && p.segments[0].name == "ram" => {
                            cart.chr_ram = true
                        }
                        _ => self.diags.error(
                            value.span,
                            "Expected a CHR asset path, a list of paths or 'ram'",
                        ),
                    }
                    for (p, span) in paths {
                        match self.asset_bytes(&p) {
                            Some(b) => {
                                if b.len() % 1024 != 0 {
                                    self.diags.error(span, format!("CHR asset '{p}' must be a multiple of 1 KiB; it has {} bytes", b.len()));
                                }
                                cart.chr.extend(b);
                            }
                            None => self.diags.error(span, format!("Missing asset '{p}'")),
                        }
                    }
                }
                "prg" => {
                    if let Some(n) = self.const_int(root, value) {
                        cart.prg_kib = Some(n as u32);
                    }
                }
                "sprites" => {}
                "battery" => match &value.kind {
                    ast::ExprKind::Bool(b) => cart.battery = *b,
                    _ => self.diags.error(value.span, "Expected true or false"),
                },
                other => {
                    if let Some(n) = self.const_int(root, value) {
                        cart.fields.insert(other.to_owned(), n);
                    }
                }
            }
        }
        self.cartridge = cart;
    }

    pub(crate) fn asset_bytes(&self, path: &str) -> Option<Vec<u8>> {
        // Asset paths resolve relative to the project root; a leading `./`
        // is accepted.
        let path = path.trim_start_matches("./");
        self.project.assets.get(path).cloned()
    }

    // -------------------------------------------------------------- driving

    fn check_all(&mut self) {
        self.check_cartridge();
        let defs: Vec<Def> = self.item_of.keys().copied().collect();
        for d in &defs {
            self.ensure(*d);
        }
        for d in defs {
            if let Def::Fn(f) = d {
                self.body(f);
            }
        }
        self.validate_entry_points();
    }

    fn validate_entry_points(&mut self) {
        let mut seen: BTreeMap<&str, Span> = BTreeMap::new();
        for (i, f) in self.functions.iter().enumerate() {
            let Some(f) = f else { continue };
            let kind = if f.attrs.reset {
                "reset"
            } else {
                match f.attrs.interrupt {
                    Some(Context::Nmi) => "nmi",
                    Some(Context::Irq) => "irq",
                    _ => continue,
                }
            };
            if let Some(prev) = seen.insert(kind, f.span) {
                self.diags.push(
                    Diagnostic::error(f.span, format!("Only one @{kind} handler is allowed"))
                        .note(prev, "previous handler"),
                );
            }
            if !f.params.is_empty() || f.result != Type::Void {
                self.diags.error(
                    self.fn_meta[i].2,
                    format!("@{kind} handlers take no parameters and return nothing"),
                );
            }
        }
    }

    fn finish(self) -> Analysis {
        let functions: Vec<Function> = self
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| match f {
                Some(f) => (**f).clone(),
                None => Function {
                    name: self.fn_meta[i].0.clone(),
                    path: self.fn_meta[i].1.clone(),
                    params: Vec::new(),
                    result: Type::Error,
                    locals: Vec::new(),
                    body: Vec::new(),
                    attrs: FnAttrs::default(),
                    span: self.fn_meta[i].2,
                    module: self.fn_meta[i].3,
                    doc: String::new(),
                    has_asm: false,
                },
            })
            .collect();
        let aliases = self
            .alias_names
            .iter()
            .zip(&self.alias_types)
            .map(|(n, t)| (n.clone(), t.clone().unwrap_or(Type::Error)))
            .collect();
        Analysis {
            sources: self.sources,
            program: Program {
                types: self.types,
                globals: self.globals,
                functions,
                cartridge: self.cartridge,
                modules: self
                    .modules
                    .iter()
                    .map(|m| module_path_of_file(&m.path))
                    .collect(),
            },
            diagnostics: self.diags,
            modules: self.modules,
            refs: self.refs,
            defs: self.defs,
            hints: self.hints,
            aliases,
        }
    }
}
