//! Semantic types.
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StructId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnumId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RangeId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    U8,
    I8,
    U16,
    I16,
    Bool,
    /// An integer literal or constant expression that has not yet been given
    /// a concrete width.
    Int,
    Void,
    Range(RangeId),
    Enum(EnumId),
    Struct(StructId),
    Array(Box<Type>, u32),
    /// Read-only view of constant ROM data.
    Slice(Box<Type>),
    /// A function never returns normally from this expression.
    Never,
    Error,
}

#[derive(Clone, Debug)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub span: crate::source::Span,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub name: String,
    pub repr: Type,
    pub variants: Vec<(String, i64)>,
    pub span: crate::source::Span,
}

#[derive(Clone, Debug)]
pub struct RangeDef {
    pub name: String,
    pub base: Type,
    /// Inclusive bounds.
    pub lo: i64,
    pub hi: i64,
    pub span: crate::source::Span,
}

#[derive(Default, Clone, Debug)]
pub struct TypeTable {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub ranges: Vec<RangeDef>,
}

impl Type {
    pub fn is_error(&self) -> bool {
        matches!(self, Type::Error)
    }
    pub fn is_machine_int(&self) -> bool {
        matches!(self, Type::U8 | Type::I8 | Type::U16 | Type::I16)
    }
}

impl TypeTable {
    /// The machine integer a scalar type is represented as.
    pub fn base(&self, t: &Type) -> Type {
        match t {
            Type::Range(r) => self.ranges[r.0 as usize].base.clone(),
            Type::Enum(e) => self.enums[e.0 as usize].repr.clone(),
            Type::Bool => Type::U8,
            t => t.clone(),
        }
    }
    /// Integer types participating in arithmetic: machine ints, ranges, untyped.
    pub fn is_integer(&self, t: &Type) -> bool {
        matches!(
            t,
            Type::U8 | Type::I8 | Type::U16 | Type::I16 | Type::Int | Type::Range(_)
        )
    }
    pub fn is_scalar(&self, t: &Type) -> bool {
        matches!(
            t,
            Type::U8
                | Type::I8
                | Type::U16
                | Type::I16
                | Type::Bool
                | Type::Int
                | Type::Range(_)
                | Type::Enum(_)
        )
    }
    pub fn signed(&self, t: &Type) -> bool {
        matches!(self.base(t), Type::I8 | Type::I16)
    }
    /// Size in bytes of one value.
    pub fn size(&self, t: &Type) -> usize {
        match t {
            Type::U8 | Type::I8 | Type::Bool => 1,
            Type::U16 | Type::I16 => 2,
            Type::Int => 2,
            Type::Range(_) | Type::Enum(_) => self.size(&self.base(t)),
            Type::Struct(s) => self.structs[s.0 as usize]
                .fields
                .iter()
                .map(|(_, t)| self.size(t))
                .sum(),
            Type::Array(t, n) => self.size(t) * *n as usize,
            Type::Slice(_) => 3,
            Type::Void | Type::Never | Type::Error => 0,
        }
    }
    /// Inclusive value bounds of a scalar type.
    pub fn bounds(&self, t: &Type) -> (i64, i64) {
        match t {
            Type::U8 => (0, 255),
            Type::I8 => (-128, 127),
            Type::U16 => (0, 65535),
            Type::I16 => (-32768, 32767),
            Type::Bool => (0, 1),
            Type::Range(r) => {
                let r = &self.ranges[r.0 as usize];
                (r.lo, r.hi)
            }
            Type::Enum(e) => {
                let e = &self.enums[e.0 as usize];
                let lo = e.variants.iter().map(|v| v.1).min().unwrap_or(0);
                let hi = e.variants.iter().map(|v| v.1).max().unwrap_or(0);
                (lo, hi)
            }
            _ => (i64::MIN / 4, i64::MAX / 4),
        }
    }
    /// Wrap a value to the representation of `t`.
    pub fn wrap(&self, t: &Type, v: i64) -> i64 {
        match self.base(t) {
            Type::U8 => v & 0xff,
            Type::I8 => v as i8 as i64,
            Type::U16 => v & 0xffff,
            Type::I16 => v as i16 as i64,
            _ => v,
        }
    }
    pub fn name(&self, t: &Type) -> String {
        match t {
            Type::U8 => "u8".into(),
            Type::I8 => "i8".into(),
            Type::U16 => "u16".into(),
            Type::I16 => "i16".into(),
            Type::Bool => "bool".into(),
            Type::Int => "integer".into(),
            Type::Void => "()".into(),
            Type::Never => "!".into(),
            Type::Error => "{error}".into(),
            Type::Range(r) => self.ranges[r.0 as usize].name.clone(),
            Type::Enum(e) => self.enums[e.0 as usize].name.clone(),
            Type::Struct(s) => self.structs[s.0 as usize].name.clone(),
            Type::Array(t, n) => format!("[{}; {n}]", self.name(t)),
            Type::Slice(t) => format!("&[{}]", self.name(t)),
        }
    }
    /// Scalar leaves of a type, as (path suffix, scalar type), in layout order.
    pub fn leaves(&self, t: &Type) -> Vec<(String, Type)> {
        match t {
            Type::Struct(s) => {
                let mut out = Vec::new();
                for (name, ft) in &self.structs[s.0 as usize].fields {
                    for (suffix, lt) in self.leaves(ft) {
                        out.push((format!(".{name}{suffix}"), lt));
                    }
                }
                out
            }
            t => vec![(String::new(), t.clone())],
        }
    }
}

pub struct Display<'a>(pub &'a TypeTable, pub &'a Type);
impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.name(self.1))
    }
}
