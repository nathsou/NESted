//! Source files, spans and line maps.
use std::collections::BTreeMap;

/// Index of a file in a [`SourceMap`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

/// A byte range in one source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(file: FileId, start: usize, end: usize) -> Self {
        Self {
            file,
            start: start as u32,
            end: end as u32,
        }
    }
    pub fn to(self, other: Span) -> Span {
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
    pub fn contains(self, offset: usize) -> bool {
        self.start as usize <= offset && offset <= self.end as usize
    }
    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

#[derive(Clone, Debug)]
pub struct SourceFile {
    /// Logical path, such as `main.nst` or `std/video.nst`.
    pub path: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            path: path.into(),
            text,
            line_starts,
        }
    }
    /// Zero-based line and UTF-16 column of a byte offset.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        let start = self.line_starts[line];
        let column = self.text[start..offset].chars().map(char::len_utf16).sum();
        (line, column)
    }
    /// Byte offset of a zero-based line and UTF-16 column.
    pub fn offset(&self, line: usize, column: usize) -> usize {
        let Some(&start) = self.line_starts.get(line) else {
            return self.text.len();
        };
        let end = self
            .line_starts
            .get(line + 1)
            .map(|e| e - 1)
            .unwrap_or(self.text.len());
        let mut units = 0;
        for (i, c) in self.text[start..end].char_indices() {
            if units >= column {
                return start + i;
            }
            units += c.len_utf16();
        }
        end
    }
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

/// All files taking part in a compilation, including the standard library.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    pub files: Vec<SourceFile>,
    by_path: BTreeMap<String, FileId>,
}

impl SourceMap {
    pub fn add(&mut self, path: &str, text: &str) -> FileId {
        if let Some(id) = self.by_path.get(path) {
            return *id;
        }
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile::new(path, text));
        self.by_path.insert(path.to_owned(), id);
        id
    }
    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }
    pub fn find(&self, path: &str) -> Option<FileId> {
        self.by_path.get(path).copied()
    }
    pub fn text(&self, span: Span) -> &str {
        &self.get(span.file).text[span.range()]
    }
}
