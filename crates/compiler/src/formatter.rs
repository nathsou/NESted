//! Token-aware, deterministic formatting shared by native tools and the LSP.
use crate::frontend::{lex, Token};

#[derive(Clone, Copy, Debug)]
pub struct FormatOptions {
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub line_width: usize,
}
impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            tab_size: 4,
            insert_spaces: true,
            line_width: 96,
        }
    }
}

struct Writer {
    out: String,
    line: String,
    indent: usize,
    unit: String,
}
impl Writer {
    fn flush(&mut self) {
        if self.line.trim().is_empty() {
            self.line.clear();
            return;
        }
        self.out.push_str(&self.unit.repeat(self.indent));
        self.out.push_str(self.line.trim_end());
        self.out.push('\n');
        self.line.clear();
    }
    fn space(&mut self) {
        if !self.line.is_empty() && !self.line.ends_with(char::is_whitespace) {
            self.line.push(' ');
        }
    }
    fn tight(&mut self, value: &str) {
        self.line.truncate(self.line.trim_end().len());
        self.line.push_str(value);
    }
    fn raw(&mut self, value: &str) {
        self.line.push_str(value);
    }
    fn comment(&mut self, value: &str) {
        self.space();
        self.raw(value);
        self.flush();
    }
}

fn gap_comments(gap: &str, w: &mut Writer) {
    let bytes = gap.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if gap[pos..].starts_with("//") {
            let end = gap[pos..].find('\n').map_or(gap.len(), |n| pos + n);
            w.comment(&gap[pos..end]);
            pos = end;
        } else if gap[pos..].starts_with("/*") {
            let start = pos;
            pos += 2;
            let mut depth = 1;
            while pos < bytes.len() && depth > 0 {
                if gap[pos..].starts_with("/*") {
                    depth += 1;
                    pos += 2;
                } else if gap[pos..].starts_with("*/") {
                    depth -= 1;
                    pos += 2;
                } else {
                    pos += gap[pos..].chars().next().unwrap().len_utf8();
                }
            }
            w.comment(&gap[start..pos]);
        } else {
            pos += gap[pos..].chars().next().unwrap().len_utf8();
        }
    }
}

fn matching(tokens: &[Token], start: usize, open: &str, close: &str) -> Option<usize> {
    let mut depth = 0;
    for (index, token) in tokens.iter().enumerate().skip(start) {
        if token.string {
            continue;
        }
        if token.text == open {
            depth += 1;
        }
        if token.text == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

/// Formats valid tokens while retaining the original spelling of strings,
/// comments and assembly. Incomplete lexical input is returned unchanged.
pub fn format_source(source: &str, options: FormatOptions) -> String {
    let Ok(tokens) = lex(source) else {
        return source.into();
    };
    let unit = if options.insert_spaces {
        " ".repeat(options.tab_size.clamp(1, 8))
    } else {
        "\t".into()
    };
    let width = options.line_width.clamp(40, 160);
    let mut w = Writer {
        out: String::new(),
        line: String::new(),
        indent: 0,
        unit,
    };
    let mut squares: Vec<(bool, usize)> = Vec::new();
    let mut previous = "";
    let mut previous_unary = false;
    let mut end = 0;
    let mut assembly = false;
    let mut pending_matches = 0usize;
    let mut braces = Vec::new();
    let mut i = 0;
    while i < tokens.len() && tokens[i].text != "<eof>" {
        let token = &tokens[i];
        let text = token.text.as_str();
        let unary = ["+", "-"].contains(&text)
            && [
                "=", "(", "[", ",", "return", "=>", "+", "-", "*", "/", "==", "!=", "<", ">",
            ]
            .contains(&previous);
        let gap = &source[end..token.span.start];
        if gap.contains("\n\n")
            && w.line.is_empty()
            && !w.out.ends_with("\n\n")
            && !w.out.is_empty()
        {
            w.out.push('\n');
        }
        gap_comments(gap, &mut w);
        let spelling = &source[token.span.start..token.span.end];
        if token.string {
            if !["(", "[", "@", "!", "~", "-", "+"].contains(&previous) {
                w.space();
            }
            w.raw(spelling);
        } else {
            match text {
                "asm" => {
                    w.space();
                    w.raw(text);
                    assembly = true;
                }
                "match" => {
                    w.space();
                    w.raw(text);
                    pending_matches += 1;
                }
                "{" if assembly => {
                    let Some(close) = matching(&tokens, i, "{", "}") else {
                        return source.into();
                    };
                    w.space();
                    w.raw("{");
                    w.flush();
                    w.indent += 1;
                    let raw = &source[token.span.end..tokens[close].span.start];
                    let lines = raw.lines().collect::<Vec<_>>();
                    let common = lines
                        .iter()
                        .filter(|l| !l.trim().is_empty())
                        .map(|l| l.len() - l.trim_start().len())
                        .min()
                        .unwrap_or(0);
                    for line in lines {
                        if line.trim().is_empty() {
                            continue;
                        }
                        w.raw(line.get(common..).unwrap_or(line));
                        w.flush();
                    }
                    w.indent -= 1;
                    w.raw("}");
                    end = tokens[close].span.end;
                    i = close + 1;
                    previous = "}";
                    assembly = false;
                    continue;
                }
                "{" => {
                    braces.push(pending_matches > 0);
                    pending_matches = pending_matches.saturating_sub(1);
                    w.space();
                    w.raw("{");
                    w.flush();
                    w.indent += 1;
                }
                "}" => {
                    braces.pop();
                    w.flush();
                    w.indent = w.indent.saturating_sub(1);
                    w.raw("}");
                    let next = tokens.get(i + 1).map(|t| t.text.as_str()).unwrap_or("");
                    if !["else", ";", ",", ")", "]"].contains(&next) {
                        w.flush();
                    }
                }
                ";" => {
                    w.tight(";");
                    if squares.is_empty() {
                        w.flush();
                    } else {
                        w.space();
                    }
                }
                "[" => {
                    if previous == "=" {
                        w.space();
                    }
                    let multiline = previous == "="
                        && matching(&tokens, i, "[", "]").is_some_and(|close| {
                            tokens[i..close].iter().any(|t| t.text == ",")
                                && (tokens[close].span.end - token.span.start + w.line.len()
                                    > width
                                    || source[token.span.start..tokens[close].span.end]
                                        .contains('\n'))
                        });
                    w.raw("[");
                    squares.push((multiline, 0));
                    if multiline {
                        w.flush();
                        w.indent += 1;
                    }
                }
                "]" => {
                    if let Some((true, _)) = squares.pop() {
                        w.flush();
                        w.indent = w.indent.saturating_sub(1);
                    }
                    w.tight("]");
                }
                "(" => {
                    if [
                        "if", "while", "match", "return", "=", "+=", "-=", "*=", "/=", "%=", "&=",
                        "|=", "^=", "<<=", ">>=", "*", "/", "%", "&", "|", "^", "<<", ">>", "==",
                        "!=", "<", "<=", ">", ">=", "&&", "||", "=>", ",",
                    ]
                    .contains(&previous)
                        || (["+", "-"].contains(&previous) && !previous_unary)
                    {
                        w.space();
                    } else {
                        w.line.truncate(w.line.trim_end().len());
                    }
                    w.raw("(");
                }
                ")" => w.tight(")"),
                "," => {
                    w.tight(",");
                    if let Some((true, count)) = squares.last_mut() {
                        *count += 1;
                        if *count >= 8 || w.line.len() + w.indent * options.tab_size >= width - 10 {
                            w.flush();
                            *count = 0;
                        } else {
                            w.space();
                        }
                    } else if braces.last() == Some(&true) {
                        w.flush();
                    } else {
                        w.space();
                    }
                }
                ":" => {
                    w.tight(":");
                    w.space();
                }
                ".." | "..=" | "." => w.tight(text),
                "@" => {
                    w.space();
                    w.raw("@");
                }
                "!" | "~" => {
                    w.space();
                    w.raw(text);
                }
                "+" | "-"
                    if [
                        "=", "(", "[", ",", "return", "=>", "+", "-", "*", "/", "==", "!=", "<",
                        ">",
                    ]
                    .contains(&previous) =>
                {
                    if !["(", "["].contains(&previous) {
                        w.space();
                    }
                    w.raw(text);
                }
                "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "<<=" | ">>="
                | "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>" | "==" | "!="
                | "<" | "<=" | ">" | ">=" | "&&" | "||" | "->" | "=>" => {
                    w.space();
                    w.raw(text);
                    w.space();
                }
                _ => {
                    if !["(", "[", "@", "!", "~", "..", "..=", "."].contains(&previous)
                        && !(["-", "+"].contains(&previous) && w.line.ends_with(previous))
                    {
                        w.space();
                    }
                    w.raw(spelling);
                }
            }
        }
        end = token.span.end;
        previous = text;
        previous_unary = unary;
        i += 1;
    }
    gap_comments(&source[end..], &mut w);
    w.flush();
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expands_blocks_and_preserves_quoted_content() {
        let source = "fn update(){let x:u8=1;if x==1{text(0,0,\" {  a;\\n   b } \");}else{x+=2;}}";
        let formatted = format_source(source, FormatOptions::default());
        assert!(formatted.contains("    let x: u8 = 1;\n    if x == 1 {\n"));
        assert!(formatted.contains("\" {  a;\\n   b } \""));
        assert!(formatted.contains("    } else {\n"));
        assert_eq!(
            format_source(&formatted, FormatOptions::default()),
            formatted
        );
    }
    #[test]
    fn assembly_and_comments_retain_their_meaning() {
        let source = "// test\nfn f()->u8{return asm(a=1){\n    lda #$ff ; instruction\nloop:\n    bne loop\n};} /* { nested /* } */ */";
        let output = format_source(source, FormatOptions::default());
        assert!(output.contains("lda #$ff ; instruction"));
        assert!(output.contains("/* { nested /* } */ */"));
        assert_eq!(format_source(&output, FormatOptions::default()), output);
    }
    #[test]
    fn wraps_lists_without_changing_types_repeats_or_indices() {
        let source = "var a:[u8;64]=[0;64];const notes:[u16;16]=[100,200,300,400,500,600,700,800,900,1000,1100,1200,1300,1400,1500,1600];fn update(){a[raw 1+2]=notes[3];}";
        let output = format_source(source, FormatOptions::default());
        assert!(output.contains("var a: [u8; 64] = [0; 64];"));
        assert!(output.contains("800,\n"));
        assert!(output.contains("a[raw 1 + 2] = notes[3];"));
        assert_eq!(format_source(&output, FormatOptions::default()), output);
    }
    #[test]
    fn respects_tabs_and_leaves_unterminated_input_intact() {
        let options = FormatOptions {
            tab_size: 2,
            insert_spaces: false,
            ..FormatOptions::default()
        };
        assert_eq!(
            format_source("fn update(){return;}", options),
            "fn update() {\n\treturn;\n}\n"
        );
        let source = "fn update(){text(0,0,\"unfinished";
        assert_eq!(format_source(source, options), source);
    }
}
