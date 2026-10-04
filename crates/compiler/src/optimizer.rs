use crate::frontend::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Default)]
pub struct Optimizations {
    pub folded: usize,
    pub simplified: usize,
    pub inlined: usize,
    pub branches: usize,
    pub unreachable: usize,
    pub unrolled: usize,
    pub dead_functions: usize,
}
fn pure(e: &Expr, volatile: &BTreeSet<String>) -> bool {
    match &e.kind {
        ExprKind::Number(_) => true,
        ExprKind::Name(n) => !volatile.contains(n),
        ExprKind::Index(..) => false,
        ExprKind::Binary(_, a, b) => pure(a, volatile) && pure(b, volatile),
        ExprKind::Unary(_, a) => pure(a, volatile),
        ExprKind::Match(value, arms) => {
            pure(value, volatile)
                && arms.iter().all(
                    |arm| matches!(&arm.body,MatchBody::Value(result) if pure(result,volatile)),
                )
        }
        ExprKind::Call(n, v) => {
            ["u8", "i8", "u16", "i16", "bool"].contains(&n.as_str())
                && v.iter().all(|e| pure(e, volatile))
        }
        _ => false,
    }
}
fn cost(e: &Expr) -> usize {
    match &e.kind {
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b, _) => 1 + cost(a) + cost(b),
        ExprKind::Unary(_, a) => 1 + cost(a),
        ExprKind::Match(value, arms) => {
            1 + cost(value)
                + arms
                    .iter()
                    .map(|a| match &a.body {
                        MatchBody::Value(v) => cost(v),
                        MatchBody::Block(b) => b.len() * 4,
                    })
                    .sum::<usize>()
        }
        ExprKind::Call(_, v) => 1 + v.iter().map(cost).sum::<usize>(),
        _ => 1,
    }
}
fn substitute(e: &mut Expr, args: &BTreeMap<String, Expr>) {
    if let ExprKind::Name(n) = &e.kind {
        if let Some(a) = args.get(n) {
            *e = a.clone();
            return;
        }
    }
    match &mut e.kind {
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b, _) => {
            substitute(a, args);
            substitute(b, args);
        }
        ExprKind::Unary(_, a) => substitute(a, args),
        ExprKind::Match(value, arms) => {
            substitute(value, args);
            for arm in arms {
                match &mut arm.body {
                    MatchBody::Value(result) => substitute(result, args),
                    MatchBody::Block(body) => replace_statements(body, args),
                }
            }
        }
        ExprKind::Call(_, v) => {
            for a in v {
                substitute(a, args);
            }
        }
        _ => {}
    }
}
fn expression(
    e: &mut Expr,
    constants: &BTreeMap<String, i64>,
    inline: &BTreeMap<String, (Vec<String>, Expr)>,
    stats: &mut Optimizations,
    volatile: &BTreeSet<String>,
) {
    match &mut e.kind {
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b, _) => {
            expression(a, constants, inline, stats, volatile);
            expression(b, constants, inline, stats, volatile);
        }
        ExprKind::Unary(_, a) => expression(a, constants, inline, stats, volatile),
        ExprKind::Match(value, arms) => {
            expression(value, constants, inline, stats, volatile);
            for arm in arms {
                match &mut arm.body {
                    MatchBody::Value(result) => {
                        expression(result, constants, inline, stats, volatile)
                    }
                    MatchBody::Block(body) => block(body, constants, inline, stats, volatile),
                }
            }
        }
        ExprKind::Call(_, v) => {
            for a in v {
                expression(a, constants, inline, stats, volatile);
            }
        }
        ExprKind::Asm(v, _) => {
            for (_, a) in v {
                expression(a, constants, inline, stats, volatile);
            }
        }
        _ => {}
    }
    if let ExprKind::Call(n, args) = &e.kind {
        if let Some((params, body)) = inline.get(n) {
            if args.iter().all(|e| pure(e, volatile)) {
                let bind = params.iter().cloned().zip(args.iter().cloned()).collect();
                let mut body = body.clone();
                substitute(&mut body, &bind);
                body.span = e.span;
                *e = body;
                stats.inlined += 1;
            }
        }
    }
    if !matches!(e.kind, ExprKind::Number(_)) {
        if let Some(n) = eval_const(e, constants) {
            e.kind = ExprKind::Number(n);
            stats.folded += 1;
            return;
        }
    }
    if let ExprKind::Binary(op, a, b) = &mut e.kind {
        let right = eval_const(b, constants);
        let left = eval_const(a, constants);
        let identity = matches!(
            (op.as_str(), right),
            ("+" | "-" | "|" | "^" | "<<" | ">>", Some(0)) | ("*" | "/", Some(1))
        ) || (op == "&" && right == Some(a.ty.mask()));
        if identity {
            let mut out = (**a).clone();
            out.span = e.span;
            *e = out;
            stats.simplified += 1;
            return;
        }
        if ((op == "*" || op == "&") && right == Some(0) && pure(a, volatile))
            || (op == "*" && left == Some(0) && pure(b, volatile))
        {
            e.kind = ExprKind::Number(0);
            stats.simplified += 1;
            return;
        }
        if let Some(n) = right {
            if n > 1 && (n as u64).is_power_of_two() {
                let shift = n.trailing_zeros() as i64;
                if op == "*" || (op == "/" && !a.ty.signed()) {
                    *op = if op == "*" { "<<" } else { ">>" }.into();
                    b.kind = ExprKind::Number(shift);
                    stats.simplified += 1;
                } else if op == "%" && !a.ty.signed() {
                    *op = "&".into();
                    b.kind = ExprKind::Number(n - 1);
                    stats.simplified += 1;
                }
            }
        }
    }
}
fn replace_statements(body: &mut [Stmt], bind: &BTreeMap<String, Expr>) {
    for s in body {
        match s {
            Stmt::Assign { target, value, .. } => {
                substitute(target, bind);
                substitute(value, bind);
            }
            Stmt::Expr(e) => substitute(e, bind),
            Stmt::Local { value, .. } => substitute(value, bind),
            Stmt::If { condition, yes, no } => {
                substitute(condition, bind);
                replace_statements(yes, bind);
                replace_statements(no, bind);
            }
            Stmt::While { condition, body } => {
                substitute(condition, bind);
                replace_statements(body, bind);
            }
            Stmt::For {
                start, end, body, ..
            } => {
                substitute(start, bind);
                substitute(end, bind);
                replace_statements(body, bind);
            }
            Stmt::Loop(body) => replace_statements(body, bind),
            Stmt::Return(Some(value), _) => substitute(value, bind),
            _ => {}
        }
    }
}
fn block(
    body: &mut Vec<Stmt>,
    constants: &BTreeMap<String, i64>,
    inline: &BTreeMap<String, (Vec<String>, Expr)>,
    stats: &mut Optimizations,
    volatile: &BTreeSet<String>,
) {
    let mut out = Vec::new();
    let mut ended = false;
    for mut s in std::mem::take(body) {
        if ended {
            stats.unreachable += 1;
            continue;
        }
        match &mut s {
            Stmt::Local { value, .. } | Stmt::Expr(value) => {
                expression(value, constants, inline, stats, volatile)
            }
            Stmt::Assign { target, value, .. } => {
                expression(value, constants, inline, stats, volatile);
                if let ExprKind::Index(_, index, _) = &mut target.kind {
                    expression(index, constants, inline, stats, volatile);
                }
            }
            Stmt::If { condition, yes, no } => {
                expression(condition, constants, inline, stats, volatile);
                block(yes, constants, inline, stats, volatile);
                block(no, constants, inline, stats, volatile);
                if let Some(v) = eval_const(condition, constants) {
                    let chosen = if v != 0 {
                        std::mem::take(yes)
                    } else {
                        std::mem::take(no)
                    };
                    stats.branches += 1;
                    ended = chosen.last().is_some_and(|s| {
                        matches!(s, Stmt::Return(..) | Stmt::Break(..) | Stmt::Continue(..))
                    });
                    out.extend(chosen);
                    continue;
                }
            }
            Stmt::While { condition, body } => {
                expression(condition, constants, inline, stats, volatile);
                block(body, constants, inline, stats, volatile);
                if eval_const(condition, constants) == Some(0) {
                    stats.branches += 1;
                    continue;
                }
            }
            Stmt::For {
                name,
                start,
                end,
                body,
                ..
            } => {
                expression(start, constants, inline, stats, volatile);
                expression(end, constants, inline, stats, volatile);
                block(body, constants, inline, stats, volatile);
                if let (Some(a), Some(b)) =
                    (eval_const(start, constants), eval_const(end, constants))
                {
                    let simple=body.iter().all(|s|matches!(s,Stmt::Assign{..}|Stmt::Expr(_)))&&!body.iter().any(|s|matches!(s,Stmt::Assign{target:Expr{kind:ExprKind::Name(n),..},..} if n==name));
                    if b >= a && b - a <= 4 && simple && body.len() <= 5 {
                        for n in a..b {
                            let mut cloned = body.clone();
                            let replacement = Expr {
                                kind: ExprKind::Number(n),
                                span: start.span,
                                ty: start.ty.clone(),
                            };
                            replace_statements(
                                &mut cloned,
                                &BTreeMap::from([(name.clone(), replacement)]),
                            );
                            block(&mut cloned, constants, inline, stats, volatile);
                            out.extend(cloned);
                        }
                        stats.unrolled += 1;
                        continue;
                    }
                }
            }
            Stmt::Loop(body) => block(body, constants, inline, stats, volatile),
            Stmt::Return(Some(e), _) => expression(e, constants, inline, stats, volatile),
            _ => {}
        }
        ended = matches!(s, Stmt::Return(..) | Stmt::Break(..) | Stmt::Continue(..));
        out.push(s);
    }
    *body = out;
}
pub fn calls_expr(e: &Expr, calls: &mut BTreeSet<String>, asm: &mut bool) {
    match &e.kind {
        ExprKind::Call(n, v) => {
            calls.insert(n.clone());
            for e in v {
                calls_expr(e, calls, asm);
            }
        }
        ExprKind::Asm(v, body) => {
            *asm = true;
            for line in body.lines() {
                let code = line.split(';').next().unwrap_or("");
                let code = code.split("//").next().unwrap_or("");
                let code = code.rsplit_once(':').map(|(_, s)| s).unwrap_or(code);
                let mut words = code.split_whitespace();
                let op = words.next().unwrap_or("");
                if op.eq_ignore_ascii_case("jsr") || op.eq_ignore_ascii_case("jmp") {
                    if let Some(target) = words.next() {
                        if target
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_')
                        {
                            calls.insert(target.trim_start_matches("__fn_").to_owned());
                        }
                    }
                }
            }
            for (_, e) in v {
                calls_expr(e, calls, asm);
            }
        }
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b, _) => {
            calls_expr(a, calls, asm);
            calls_expr(b, calls, asm);
        }
        ExprKind::Unary(_, e) => calls_expr(e, calls, asm),
        ExprKind::Match(value, arms) => {
            calls_expr(value, calls, asm);
            for arm in arms {
                match &arm.body {
                    MatchBody::Value(result) => calls_expr(result, calls, asm),
                    MatchBody::Block(body) => calls_block(body, calls, asm),
                }
            }
        }
        _ => {}
    }
}
pub fn calls_block(body: &[Stmt], calls: &mut BTreeSet<String>, asm: &mut bool) {
    for s in body {
        match s {
            Stmt::Local { value, .. } | Stmt::Expr(value) => calls_expr(value, calls, asm),
            Stmt::Assign { target, value, .. } => {
                calls_expr(target, calls, asm);
                calls_expr(value, calls, asm);
            }
            Stmt::If { condition, yes, no } => {
                calls_expr(condition, calls, asm);
                calls_block(yes, calls, asm);
                calls_block(no, calls, asm);
            }
            Stmt::While { condition, body } => {
                calls_expr(condition, calls, asm);
                calls_block(body, calls, asm);
            }
            Stmt::For {
                start, end, body, ..
            } => {
                calls_expr(start, calls, asm);
                calls_expr(end, calls, asm);
                calls_block(body, calls, asm);
            }
            Stmt::Loop(body) => calls_block(body, calls, asm),
            Stmt::Return(Some(e), _) => calls_expr(e, calls, asm),
            _ => {}
        }
    }
}
pub fn optimize(program: &mut Program) -> Optimizations {
    let volatile = program
        .globals
        .iter()
        .filter(|g| g.attrs.contains_key("shared"))
        .map(|g| g.name.clone())
        .collect::<BTreeSet<_>>();
    let mut stats = Optimizations::default();
    let mut constants = BTreeMap::new();
    for g in &program.globals {
        if g.constant {
            if let Initial::Scalar(e) = &g.initial {
                if let Some(n) = eval_const(e, &constants) {
                    constants.insert(g.name.clone(), n);
                }
            }
        }
    }
    let inline = program
        .functions
        .iter()
        .filter_map(|f| {
            if f.attrs.contains_key("noinline") {
                return None;
            }
            if let [Stmt::Return(Some(e), _)] = f.body.as_slice() {
                if pure(e, &volatile) && cost(e) <= 16 {
                    return Some((
                        f.name.clone(),
                        (f.params.iter().map(|p| p.0.clone()).collect(), e.clone()),
                    ));
                }
            }
            None
        })
        .collect::<BTreeMap<_, _>>();
    for _ in 0..3 {
        for f in &mut program.functions {
            block(&mut f.body, &constants, &inline, &mut stats, &volatile);
        }
    }
    let mut reachable = program
        .functions
        .iter()
        .filter(|f| {
            ["init", "update"].contains(&f.name.as_str())
                || f.attrs
                    .keys()
                    .any(|a| ["reset", "nmi", "irq", "export"].contains(&a.as_str()))
        })
        .map(|f| f.name.clone())
        .collect::<BTreeSet<_>>();
    let mut changed = true;
    let mut asm = false;
    while changed {
        let old = reachable.len();
        for f in &program.functions {
            if reachable.contains(&f.name) {
                calls_block(&f.body, &mut reachable, &mut asm);
            }
        }
        changed = old != reachable.len();
    }
    if !asm {
        let old = program.functions.len();
        program.functions.retain(|f| reachable.contains(&f.name));
        stats.dead_functions = old - program.functions.len();
    }
    stats
}
fn expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Number(n) => format!("{n}:{}", e.ty),
        ExprKind::String(s) => format!("{s:?}"),
        ExprKind::Name(n) => format!("{n}:{}", e.ty),
        ExprKind::Index(a, b, raw) => {
            format!("{}[{}{}]", expr(a), if *raw { "raw " } else { "" }, expr(b))
        }
        ExprKind::Unary(op, e) => format!("({op} {})", expr(e)),
        ExprKind::Binary(op, a, b) => format!("({op} {} {})", expr(a), expr(b)),
        ExprKind::Call(n, v) => {
            format!("{n}({})", v.iter().map(expr).collect::<Vec<_>>().join(", "))
        }
        ExprKind::Asm(_, s) => format!("asm opaque {{ {} }}", s.trim().replace('\n', "; ")),
        ExprKind::Match(value, arms) => {
            let mut out = format!("match {} {{\n", expr(value));
            for arm in arms {
                let patterns = if arm.patterns.is_empty() {
                    "_".into()
                } else {
                    arm.patterns
                        .iter()
                        .map(|(lo, hi)| {
                            hi.as_ref().map_or_else(
                                || expr(lo),
                                |hi| format!("{}..={}", expr(lo), expr(hi)),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" | ")
                };
                out.push_str(&format!("  {patterns} => "));
                match &arm.body {
                    MatchBody::Value(v) => out.push_str(&format!("{},\n", expr(v))),
                    MatchBody::Block(body) => {
                        out.push_str("{\n");
                        dump_block(body, 2, &mut out);
                        out.push_str("  }\n");
                    }
                }
            }
            out.push('}');
            out
        }
    }
}
fn dump_block(body: &[Stmt], indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    for s in body {
        match s {
            Stmt::Local {
                name, ty, value, ..
            } => out.push_str(&format!(
                "{pad}local {name}: {} = {}\n",
                ty.as_ref().unwrap_or(&Ty::Void),
                expr(value)
            )),
            Stmt::Assign { target, op, value } => {
                out.push_str(&format!("{pad}{} {op} {}\n", expr(target), expr(value)))
            }
            Stmt::Expr(e) => out.push_str(&format!("{pad}{}\n", expr(e))),
            Stmt::If { condition, yes, no } => {
                out.push_str(&format!("{pad}if {}\n", expr(condition)));
                dump_block(yes, indent + 1, out);
                if !no.is_empty() {
                    out.push_str(&format!("{pad}else\n"));
                    dump_block(no, indent + 1, out);
                }
            }
            Stmt::While { condition, body } => {
                out.push_str(&format!("{pad}while {}\n", expr(condition)));
                dump_block(body, indent + 1, out);
            }
            Stmt::For {
                name,
                start,
                end,
                body,
                ..
            } => {
                out.push_str(&format!(
                    "{pad}for {name} in {}..{}\n",
                    expr(start),
                    expr(end)
                ));
                dump_block(body, indent + 1, out);
            }
            Stmt::Loop(body) => {
                out.push_str(&format!("{pad}loop\n"));
                dump_block(body, indent + 1, out);
            }
            Stmt::Return(e, _) => out.push_str(&format!(
                "{pad}return {}\n",
                e.as_ref().map(expr).unwrap_or_default()
            )),
            Stmt::Break(_) => out.push_str(&format!("{pad}break\n")),
            Stmt::Continue(_) => out.push_str(&format!("{pad}continue\n")),
        }
    }
}
pub fn dump(program: &Program) -> String {
    let mut out = String::new();
    for g in &program.globals {
        out.push_str(&format!(
            "{} {}: {} ({} bytes) {:?}\n",
            if g.constant { "rom" } else { "ram" },
            g.name,
            g.ty,
            g.ty.size(),
            g.attrs
        ));
    }
    for f in &program.functions {
        out.push_str(&format!(
            "\nfn {}({}) -> {} {:?}\n",
            f.name,
            f.params
                .iter()
                .map(|p| format!("{}: {}", p.0, p.1))
                .collect::<Vec<_>>()
                .join(", "),
            f.result,
            f.attrs
        ));
        dump_block(&f.body, 1, &mut out);
    }
    out
}
