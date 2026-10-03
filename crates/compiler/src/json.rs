//! A small, bounded JSON implementation shared by stdio LSP and the WASM ABI.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    pub fn object<const N: usize>(values: [(&str, Json); N]) -> Self {
        Self::Object(values.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Self::Object(v) => v.get(key).unwrap_or(&Self::Null),
            _ => &Self::Null,
        }
    }
    pub fn str(&self) -> &str {
        if let Self::String(v) = self {
            v
        } else {
            ""
        }
    }
    pub fn usize(&self) -> usize {
        if let Self::Number(v) = self {
            *v as usize
        } else {
            0
        }
    }
    pub fn bool(&self) -> bool {
        matches!(self, Self::Bool(true))
    }
    pub fn array(&self) -> &[Json] {
        if let Self::Array(v) = self {
            v
        } else {
            &[]
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    pub fn stringify(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(v) => v.to_string(),
            Self::Number(v) => {
                if v.is_finite() {
                    v.to_string()
                } else {
                    "null".into()
                }
            }
            Self::String(s) => {
                let mut out = String::from("\"");
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
                out
            }
            Self::Array(v) => format!(
                "[{}]",
                v.iter().map(Self::stringify).collect::<Vec<_>>().join(",")
            ),
            Self::Object(v) => format!(
                "{{{}}}",
                v.iter()
                    .map(|(k, v)| format!(
                        "{}:{}",
                        Self::String(k.clone()).stringify(),
                        v.stringify()
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut p = Parser { source, pos: 0 };
        let value = p.value(0)?;
        p.space();
        if p.pos != source.len() {
            return Err(format!("Trailing JSON at byte {}", p.pos));
        }
        Ok(value)
    }
}
impl From<&str> for Json {
    fn from(v: &str) -> Self {
        Self::String(v.into())
    }
}
impl From<String> for Json {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}
impl From<usize> for Json {
    fn from(v: usize) -> Self {
        Self::Number(v as f64)
    }
}
impl From<u16> for Json {
    fn from(v: u16) -> Self {
        Self::Number(v as f64)
    }
}
impl From<u8> for Json {
    fn from(v: u8) -> Self {
        Self::Number(v as f64)
    }
}
impl From<bool> for Json {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .source
            .as_bytes()
            .get(self.pos)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.pos += 1;
        }
    }
    fn byte(&mut self, c: u8) -> Result<(), String> {
        self.space();
        if self.source.as_bytes().get(self.pos) == Some(&c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("Expected '{}' at byte {}", c as char, self.pos))
        }
    }
    fn hex(&mut self) -> Result<u32, String> {
        let s = self
            .source
            .get(self.pos..self.pos + 4)
            .ok_or("Incomplete JSON escape")?;
        let n = u32::from_str_radix(s, 16).map_err(|_| "Invalid JSON escape")?;
        self.pos += 4;
        Ok(n)
    }
    fn string(&mut self) -> Result<String, String> {
        self.byte(b'"')?;
        let mut out = String::new();
        loop {
            let c = self.source[self.pos..]
                .chars()
                .next()
                .ok_or("Unterminated JSON string")?;
            self.pos += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self
                        .source
                        .as_bytes()
                        .get(self.pos)
                        .copied()
                        .ok_or("Incomplete JSON escape")?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut n = self.hex()?;
                            if (0xd800..=0xdbff).contains(&n) {
                                if !self.source[self.pos..].starts_with("\\u") {
                                    return Err("Missing low surrogate".into());
                                }
                                self.pos += 2;
                                let lo = self.hex()?;
                                if !(0xdc00..=0xdfff).contains(&lo) {
                                    return Err("Invalid low surrogate".into());
                                }
                                n = 0x10000 + ((n - 0xd800) << 10) + lo - 0xdc00;
                            }
                            out.push(char::from_u32(n).ok_or("Invalid Unicode scalar")?);
                        }
                        _ => return Err("Invalid JSON escape".into()),
                    }
                }
                c if c < ' ' => return Err("Control character in JSON string".into()),
                c => out.push(c),
            }
        }
    }
    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 128 {
            return Err("JSON nesting exceeds 128".into());
        }
        self.space();
        let b = self
            .source
            .as_bytes()
            .get(self.pos)
            .copied()
            .ok_or("Expected JSON value")?;
        match b {
            b'"' => Ok(Json::String(self.string()?)),
            b'[' => {
                self.pos += 1;
                self.space();
                let mut values = Vec::new();
                if self.source.as_bytes().get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return Ok(Json::Array(values));
                }
                loop {
                    values.push(self.value(depth + 1)?);
                    self.space();
                    if self.source.as_bytes().get(self.pos) == Some(&b']') {
                        self.pos += 1;
                        break;
                    }
                    self.byte(b',')?;
                }
                Ok(Json::Array(values))
            }
            b'{' => {
                self.pos += 1;
                self.space();
                let mut values = BTreeMap::new();
                if self.source.as_bytes().get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                    return Ok(Json::Object(values));
                }
                loop {
                    let key = self.string()?;
                    self.byte(b':')?;
                    values.insert(key, self.value(depth + 1)?);
                    self.space();
                    if self.source.as_bytes().get(self.pos) == Some(&b'}') {
                        self.pos += 1;
                        break;
                    }
                    self.byte(b',')?;
                }
                Ok(Json::Object(values))
            }
            b't' if self.source[self.pos..].starts_with("true") => {
                self.pos += 4;
                Ok(Json::Bool(true))
            }
            b'f' if self.source[self.pos..].starts_with("false") => {
                self.pos += 5;
                Ok(Json::Bool(false))
            }
            b'n' if self.source[self.pos..].starts_with("null") => {
                self.pos += 4;
                Ok(Json::Null)
            }
            b'-' | b'0'..=b'9' => {
                let start = self.pos;
                if b == b'-' {
                    self.pos += 1;
                }
                match self.source.as_bytes().get(self.pos) {
                    Some(b'0') => self.pos += 1,
                    Some(b'1'..=b'9') => {
                        while self
                            .source
                            .as_bytes()
                            .get(self.pos)
                            .is_some_and(u8::is_ascii_digit)
                        {
                            self.pos += 1;
                        }
                    }
                    _ => return Err("Invalid JSON number".into()),
                }
                if self.source.as_bytes().get(self.pos) == Some(&b'.') {
                    self.pos += 1;
                    let begin = self.pos;
                    while self
                        .source
                        .as_bytes()
                        .get(self.pos)
                        .is_some_and(u8::is_ascii_digit)
                    {
                        self.pos += 1;
                    }
                    if begin == self.pos {
                        return Err("Invalid JSON fraction".into());
                    }
                }
                if matches!(self.source.as_bytes().get(self.pos), Some(b'e' | b'E')) {
                    self.pos += 1;
                    if matches!(self.source.as_bytes().get(self.pos), Some(b'+' | b'-')) {
                        self.pos += 1;
                    }
                    let begin = self.pos;
                    while self
                        .source
                        .as_bytes()
                        .get(self.pos)
                        .is_some_and(u8::is_ascii_digit)
                    {
                        self.pos += 1;
                    }
                    if begin == self.pos {
                        return Err("Invalid JSON exponent".into());
                    }
                }
                let n: f64 = self.source[start..self.pos]
                    .parse()
                    .map_err(|_| "Invalid JSON number")?;
                if !n.is_finite() {
                    return Err("JSON number is not finite".into());
                }
                Ok(Json::Number(n))
            }
            _ => Err(format!("Invalid JSON at byte {}", self.pos)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_roundtrip() {
        let v =
            Json::parse(r#"{"text":"\uD83C\uDFB8\n音楽","n":-1.25e2,"a":[true,null]}"#).unwrap();
        assert_eq!(v.get("text").str(), "🎸\n音楽");
        assert_eq!(Json::parse(&v.stringify()).unwrap(), v);
    }
    #[test]
    fn rejects_malformed() {
        for s in [
            "01",
            "[1,]",
            "{\"x\":}",
            "1e",
            "\"\\uD800\"",
            "true false",
            "1e999",
        ] {
            assert!(Json::parse(s).is_err(), "{s}");
        }
    }
}
