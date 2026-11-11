//! Diagnostics.
use crate::source::{SourceMap, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Hint,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub span: Span,
    pub message: String,
    /// Secondary locations with explanations.
    pub notes: Vec<(Span, String)>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            span,
            message: message.into(),
            notes: Vec::new(),
        }
    }
    pub fn warning(span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(span, message)
        }
    }
    pub fn note(mut self, span: Span, message: impl Into<String>) -> Self {
        self.notes.push((span, message.into()));
        self
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
    /// `path:line:column: error: message`.
    pub fn render(&self, sources: &SourceMap) -> String {
        let file = sources.get(self.span.file);
        let (line, column) = file.position(self.span.start as usize);
        let kind = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Hint => "hint",
        };
        let mut out = format!(
            "{}:{}:{}: {kind}: {}",
            file.path,
            line + 1,
            column + 1,
            self.message
        );
        let text = file.text.lines().nth(line).unwrap_or("");
        if !text.is_empty() {
            let width = (self.span.end - self.span.start).max(1) as usize;
            let width = width.min(text.len().saturating_sub(column).max(1));
            out.push_str(&format!(
                "\n  {text}\n  {}{}",
                " ".repeat(column),
                "^".repeat(width)
            ));
        }
        for (span, note) in &self.notes {
            let file = sources.get(span.file);
            let (line, column) = file.position(span.start as usize);
            out.push_str(&format!(
                "\n  note: {}:{}:{}: {note}",
                file.path,
                line + 1,
                column + 1
            ));
        }
        out
    }
}

#[derive(Default, Debug, Clone)]
pub struct Diagnostics {
    pub items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn push(&mut self, d: Diagnostic) {
        self.items.push(d);
    }
    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::error(span, message));
    }
    pub fn warning(&mut self, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::warning(span, message));
    }
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }
    pub fn extend(&mut self, other: Diagnostics) {
        self.items.extend(other.items);
    }
}
