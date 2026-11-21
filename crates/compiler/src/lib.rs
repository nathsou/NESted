//! NESted: a statically allocated language for the NES, with an embeddable
//! compiler, assembler, linker and language service. No third-party
//! dependencies and no I/O: hosts supply sources and asset bytes.
// Passes walk parallel per-block tables by index and thread a lot of
// context through helpers; these lints fight that style.
#![allow(
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::collapsible_match
)]
pub mod asm;
pub mod ast;
pub mod build_mir;
pub mod check;
pub mod check_expr;
pub mod codegen;
pub mod consteval;
pub mod diag;
pub mod fmt;
pub mod hir;
pub mod json;
pub mod lexer;
pub mod link;
pub mod lsp;
pub mod mir;
pub mod parser;
pub mod source;
pub mod stdlib;
pub mod types;

pub use check::{analyze, Analysis, Project};
pub use diag::{Diagnostic, Diagnostics, Severity};
pub use source::SourceMap;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub optimize: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { optimize: true }
    }
}

/// A named intermediate representation, for inspection.
#[derive(Clone, Debug)]
pub struct Pass {
    pub name: String,
    pub text: String,
}

pub struct Output {
    pub rom: Vec<u8>,
    pub analysis: Analysis,
    pub mir: mir::Mir,
    pub linked: link::Linked,
    pub board: link::Board,
    pub passes: Vec<Pass>,
    pub warnings: Vec<Diagnostic>,
}

pub struct Failure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
    /// Intermediate passes produced before the failure.
    pub passes: Vec<Pass>,
}

impl Failure {
    pub fn render(&self) -> String {
        self.diagnostics
            .iter()
            .map(|d| d.render(&self.sources))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl std::fmt::Debug for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

pub fn compile(project: &Project, options: Options) -> Result<Output, Failure> {
    let analysis = analyze(project);
    let fail = |analysis: Analysis, mut extra: Vec<Diagnostic>| {
        let mut d: Vec<Diagnostic> = analysis.diagnostics.items.clone();
        d.append(&mut extra);
        Failure {
            diagnostics: d,
            sources: analysis.sources,
            passes: Vec::new(),
        }
    };
    if analysis.diagnostics.has_errors() {
        return Err(fail(analysis, Vec::new()));
    }
    let mut diags = Diagnostics::default();
    let mut mir = build_mir::build(&analysis.program, &mut diags);
    if diags.has_errors() {
        return Err(fail(analysis, diags.items));
    }
    let mut passes = vec![Pass {
        name: "MIR".into(),
        text: mir.dump(),
    }];
    if options.optimize {
        opt::optimize(&mut mir);
        passes.push(Pass {
            name: "MIR optimized".into(),
            text: mir.dump(),
        });
    }
    opt::prepare(&mut mir, &mut diags);
    if diags.has_errors() {
        return Err(fail(analysis, diags.items));
    }
    let board =
        link::Board::from_name(&analysis.program.cartridge.mapper).unwrap_or(link::Board::Nrom);
    let mut units = Vec::new();
    let mut asm_text = String::new();
    for i in 0..mir.insts.len() {
        let id = mir::InstId(i as u32);
        if mir.insts[i].blocks.is_empty() {
            continue;
        }
        let mut out = codegen::generate(&mir, id, board.banking());
        for (span, msg) in out.errors.drain(..) {
            diags.error(span, msg);
        }
        if options.optimize {
            peephole::optimize(&mut out.lines, &mir, id);
        }
        asm_text.push_str(&format!("; {}\n", mir.insts[i].name));
        for l in &out.lines {
            match l {
                asm::Line::Ins(ins) => asm_text.push_str(&format!("    {ins}\n")),
                asm::Line::Label(n) => asm_text.push_str(&format!("{n}:\n")),
                _ => {}
            }
        }
        units.push(link::Unit {
            inst: id,
            lines: out.lines,
            extra_vars: out.extra_vars,
            helpers: out.helpers,
            far: out.far_calls,
        });
    }
    if diags.has_errors() {
        return Err(fail(analysis, diags.items));
    }
    passes.push(Pass {
        name: "6502".into(),
        text: asm_text,
    });
    let interference = mir.insts.iter().map(opt::interference).collect();
    let linked = match link::link(link::LinkInput {
        mir: &mir,
        interference,
        units,
        cart: &analysis.program.cartridge,
        board,
    }) {
        Ok(l) => l,
        Err(errors) => {
            let span = analysis.program.cartridge.span;
            let extra = errors
                .into_iter()
                .map(|e| Diagnostic::error(span, e))
                .collect();
            let mut f = fail(analysis, extra);
            f.passes = passes;
            return Err(f);
        }
    };
    passes.push(Pass {
        name: "Layout".into(),
        text: linked.layout.clone(),
    });
    let mut listing = String::new();
    for (bank, item) in &linked.listing {
        listing.push_str(&format!(
            "{}{:04X}  {:<9} {:<22} {}\n",
            bank.map(|b| format!("{b:02}:"))
                .unwrap_or_else(|| "   ".into()),
            item.address,
            item.bytes
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
            item.text,
            if item.cycles.0 == item.cycles.1 {
                format!("{}", item.cycles.0)
            } else {
                format!("{}-{}", item.cycles.0, item.cycles.1)
            }
        ));
    }
    passes.push(Pass {
        name: "Listing".into(),
        text: listing,
    });
    let mut warnings: Vec<Diagnostic> = analysis.diagnostics.items.clone();
    warnings.extend(diags.items);
    Ok(Output {
        rom: linked.rom.clone(),
        analysis,
        mir,
        linked,
        board,
        passes,
        warnings,
    })
}

pub mod opt;
pub mod peephole;
