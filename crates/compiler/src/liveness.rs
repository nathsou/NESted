//! Backward liveness of function locals over the structured AST.
//!
//! Recursive calls must preserve the caller's static frame. Saving only the
//! locals that can still be read after a call keeps that cost proportional to
//! the state a search-style function really carries between its children.
use crate::frontend::*;
use std::collections::{BTreeMap, BTreeSet};

pub type Live = BTreeSet<String>;

/// Live-out sets keyed by statement address. The AST is not moved while code
/// generation runs, so addresses identify statements for that duration.
pub struct Liveness {
    pub after: BTreeMap<usize, Live>,
    locals: BTreeSet<String>,
    /// Pairs of locals that are live at the same time (ordered pairs, both
    /// directions stored).
    interference: BTreeSet<(String, String)>,
    /// Locals whose every definition the analysis has seen.
    pub defined: BTreeSet<String>,
}

struct Targets {
    exit: Live,
    head: Live,
}

fn key(s: &Stmt) -> usize {
    s as *const Stmt as usize
}

impl Liveness {
    pub fn analyze(f: &Function) -> Self {
        let mut l = Self {
            after: BTreeMap::new(),
            locals: f.locals.keys().cloned().collect(),
            interference: BTreeSet::new(),
            defined: BTreeSet::new(),
        };
        let mut entry = l.block(&f.body, Live::new(), None);
        entry.extend(f.params.iter().map(|p| p.0.clone()));
        for p in &f.params {
            l.def(&p.0, &entry);
        }
        // Definitions inside match-arm blocks are not visited as statements.
        let mut hidden = BTreeSet::new();
        hidden_definitions(&f.body, false, &mut hidden);
        for n in hidden {
            l.defined.remove(&n);
        }
        l
    }
    pub fn interferes(&self, a: &str, b: &str) -> bool {
        self.interference.contains(&(a.to_owned(), b.to_owned()))
    }
    fn def(&mut self, v: &str, live: &Live) {
        self.defined.insert(v.to_owned());
        for w in live {
            if w != v {
                self.interference.insert((v.to_owned(), w.clone()));
                self.interference.insert((w.clone(), v.to_owned()));
            }
        }
    }
    pub fn reads(&self, e: &Expr, out: &mut Live) {
        match &e.kind {
            ExprKind::Name(n) if self.locals.contains(n) => {
                out.insert(n.clone());
            }
            ExprKind::Index(a, b, _) | ExprKind::Binary(_, a, b) => {
                self.reads(a, out);
                self.reads(b, out);
            }
            ExprKind::Unary(_, a) => self.reads(a, out),
            ExprKind::Call(_, v) => v.iter().for_each(|a| self.reads(a, out)),
            ExprKind::Asm(v, body) => {
                v.iter().for_each(|(_, a)| self.reads(a, out));
                // Assembly may name any local.
                for n in &self.locals {
                    if body.contains(n.as_str()) {
                        out.insert(n.clone());
                    }
                }
            }
            ExprKind::Match(value, arms) => {
                self.reads(value, out);
                for arm in arms {
                    match &arm.body {
                        MatchBody::Value(v) => self.reads(v, out),
                        MatchBody::Block(b) => self.block_reads(b, out),
                    }
                }
            }
            _ => {}
        }
    }
    fn block_reads(&self, body: &[Stmt], out: &mut Live) {
        for s in body {
            match s {
                Stmt::Local { value, .. } | Stmt::Expr(value) => self.reads(value, out),
                Stmt::Assign { target, value, .. } => {
                    self.reads(target, out);
                    self.reads(value, out);
                }
                Stmt::If { condition, yes, no } => {
                    self.reads(condition, out);
                    self.block_reads(yes, out);
                    self.block_reads(no, out);
                }
                Stmt::While { condition, body } => {
                    self.reads(condition, out);
                    self.block_reads(body, out);
                }
                Stmt::For {
                    name,
                    start,
                    end,
                    body,
                    ..
                } => {
                    out.insert(name.clone());
                    self.reads(start, out);
                    self.reads(end, out);
                    self.block_reads(body, out);
                }
                Stmt::Loop(body) => self.block_reads(body, out),
                Stmt::Return(Some(e), _) => self.reads(e, out),
                _ => {}
            }
        }
    }
    fn block(&mut self, body: &[Stmt], mut live: Live, t: Option<&Targets>) -> Live {
        for s in body.iter().rev() {
            live = self.statement(s, live, t);
        }
        live
    }
    fn statement(&mut self, s: &Stmt, out: Live, t: Option<&Targets>) -> Live {
        self.after.insert(key(s), out.clone());
        match s {
            Stmt::Local { name, value, .. } => {
                self.def(name, &out);
                let mut live = out;
                live.remove(name);
                self.reads(value, &mut live);
                live
            }
            Stmt::Assign { target, op, value } => {
                if let ExprKind::Name(n) = &target.kind {
                    if self.locals.contains(n) {
                        self.def(n, &out);
                    }
                }
                let mut live = out;
                match &target.kind {
                    ExprKind::Name(n) if op == "=" => {
                        live.remove(n);
                    }
                    _ => self.reads(target, &mut live),
                }
                self.reads(value, &mut live);
                live
            }
            Stmt::Expr(e) => {
                let mut live = out;
                self.reads(e, &mut live);
                live
            }
            Stmt::If { condition, yes, no } => {
                let mut live = self.block(yes, out.clone(), t);
                live.extend(self.block(no, out, t));
                self.reads(condition, &mut live);
                live
            }
            Stmt::While { condition, body } => {
                let mut head = out.clone();
                self.reads(condition, &mut head);
                loop {
                    let targets = Targets {
                        exit: out.clone(),
                        head: head.clone(),
                    };
                    let mut next = self.block(body, head.clone(), Some(&targets));
                    next.extend(out.iter().cloned());
                    self.reads(condition, &mut next);
                    if next.is_subset(&head) {
                        break head;
                    }
                    head.extend(next);
                }
            }
            Stmt::For {
                name,
                start,
                end,
                body,
                ..
            } => {
                // The counter is read by the test and by the increment.
                let mut head = out.clone();
                head.insert(name.clone());
                self.reads(end, &mut head);
                loop {
                    let targets = Targets {
                        exit: out.clone(),
                        head: head.clone(),
                    };
                    let mut next = self.block(body, head.clone(), Some(&targets));
                    next.extend(head.iter().cloned());
                    if next.is_subset(&head) {
                        break;
                    }
                    head.extend(next);
                }
                self.def(name, &head);
                let mut live = head;
                live.remove(name);
                self.reads(start, &mut live);
                live
            }
            Stmt::Loop(body) => {
                let mut head = Live::new();
                loop {
                    let targets = Targets {
                        exit: out.clone(),
                        head: head.clone(),
                    };
                    let next = self.block(body, head.clone(), Some(&targets));
                    if next.is_subset(&head) {
                        break head;
                    }
                    head.extend(next);
                }
            }
            Stmt::Break(_) => t.map(|t| t.exit.clone()).unwrap_or_default(),
            Stmt::Continue(_) => t.map(|t| t.head.clone()).unwrap_or_default(),
            Stmt::Return(value, _) => {
                let mut live = Live::new();
                if let Some(e) = value {
                    self.reads(e, &mut live);
                }
                live
            }
        }
    }
}

fn hidden_definitions(body: &[Stmt], hidden: bool, out: &mut BTreeSet<String>) {
    fn expr(e: &Expr, out: &mut BTreeSet<String>) {
        match &e.kind {
            ExprKind::Match(value, arms) => {
                expr(value, out);
                for arm in arms {
                    match &arm.body {
                        MatchBody::Value(v) => expr(v, out),
                        MatchBody::Block(b) => hidden_definitions(b, true, out),
                    }
                }
            }
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b, _) => {
                expr(a, out);
                expr(b, out);
            }
            ExprKind::Unary(_, a) => expr(a, out),
            ExprKind::Call(_, v) => v.iter().for_each(|a| expr(a, out)),
            _ => {}
        }
    }
    for s in body {
        match s {
            Stmt::Local { name, value, .. } => {
                if hidden {
                    out.insert(name.clone());
                }
                expr(value, out);
            }
            Stmt::Assign { target, value, .. } => {
                if hidden {
                    if let ExprKind::Name(n) = &target.kind {
                        out.insert(n.clone());
                    }
                }
                expr(target, out);
                expr(value, out);
            }
            Stmt::Expr(e) => expr(e, out),
            Stmt::If { condition, yes, no } => {
                expr(condition, out);
                hidden_definitions(yes, hidden, out);
                hidden_definitions(no, hidden, out);
            }
            Stmt::While { condition, body } => {
                expr(condition, out);
                hidden_definitions(body, hidden, out);
            }
            Stmt::For {
                name,
                start,
                end,
                body,
                ..
            } => {
                if hidden {
                    out.insert(name.clone());
                }
                expr(start, out);
                expr(end, out);
                hidden_definitions(body, hidden, out);
            }
            Stmt::Loop(body) => hidden_definitions(body, hidden, out),
            Stmt::Return(Some(e), _) => expr(e, out),
            _ => {}
        }
    }
}
