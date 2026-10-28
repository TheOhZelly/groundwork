//! Minecraft Named Binary Tag (NBT) format.
//!
//! Replaces the JNBT library used by the Java version of WorldPainter. All
//! values are big-endian and strings are encoded as Java "modified UTF-8".

mod mutf8;

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};
use flate2::Compression as FlateLevel;
use indexmap::IndexMap;
use std::fmt;
use std::io::{self, Read, Write};

pub const TAG_END: u8 = 0;
pub const TAG_BYTE: u8 = 1;
pub const TAG_SHORT: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_LONG: u8 = 4;
pub const TAG_FLOAT: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_BYTE_ARRAY: u8 = 7;
pub const TAG_STRING: u8 = 8;
pub const TAG_LIST: u8 = 9;
pub const TAG_COMPOUND: u8 = 10;
pub const TAG_INT_ARRAY: u8 = 11;
pub const TAG_LONG_ARRAY: u8 = 12;

/// Nesting limit, matching Minecraft's own protection against malicious data.
const MAX_DEPTH: usize = 512;

pub type Compound = IndexMap<String, Tag>;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(List),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// A homogeneous list. The element type is kept explicitly so that empty
/// lists round-trip with the same type id they were read with.
#[derive(Debug, Clone, PartialEq)]
pub struct List {
    pub element_type: u8,
    pub items: Vec<Tag>,
}

impl List {
    pub fn empty(element_type: u8) -> Self {
        List {
            element_type,
            items: Vec::new(),
        }
    }

    /// Builds a list from tags, which must all have the same type.
    pub fn of(items: Vec<Tag>) -> Result<Self, Error> {
        let element_type = items.first().map_or(TAG_END, Tag::type_id);
        if let Some(bad) = items.iter().find(|t| t.type_id() != element_type) {
            return Err(Error::Format(format!(
                "list of type {element_type} cannot contain tag of type {}",
                bad.type_id()
            )));
        }
        Ok(List {
            element_type,
            items,
        })
    }

    pub fn compounds(items: Vec<Compound>) -> Self {
        List {
            element_type: TAG_COMPOUND,
            items: items.into_iter().map(Tag::Compound).collect(),
        }
    }

    pub fn strings<S: Into<String>>(items: impl IntoIterator<Item = S>) -> Self {
        List {
            element_type: TAG_STRING,
            items: items.into_iter().map(|s| Tag::String(s.into())).collect(),
        }
    }
}

impl Tag {
    pub fn type_id(&self) -> u8 {
        match self {
            Tag::Byte(_) => TAG_BYTE,
            Tag::Short(_) => TAG_SHORT,
            Tag::Int(_) => TAG_INT,
            Tag::Long(_) => TAG_LONG,
            Tag::Float(_) => TAG_FLOAT,
            Tag::Double(_) => TAG_DOUBLE,
            Tag::ByteArray(_) => TAG_BYTE_ARRAY,
            Tag::String(_) => TAG_STRING,
            Tag::List(_) => TAG_LIST,
            Tag::Compound(_) => TAG_COMPOUND,
            Tag::IntArray(_) => TAG_INT_ARRAY,
            Tag::LongArray(_) => TAG_LONG_ARRAY,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(v as i64),
            Tag::Short(v) => Some(v as i64),
            Tag::Int(v) => Some(v as i64),
            Tag::Long(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_compound(&self) -> Option<&Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&List> {
        match self {
            Tag::List(l) => Some(l),
            _ => None,
        }
    }
}

impl From<bool> for Tag {
    fn from(v: bool) -> Self {
        Tag::Byte(v as i8)
    }
}
impl From<i8> for Tag {
    fn from(v: i8) -> Self {
        Tag::Byte(v)
    }
}
impl From<i16> for Tag {
    fn from(v: i16) -> Self {
        Tag::Short(v)
    }
}
impl From<i32> for Tag {
    fn from(v: i32) -> Self {
        Tag::Int(v)
    }
}
impl From<i64> for Tag {
    fn from(v: i64) -> Self {
        Tag::Long(v)
    }
}
impl From<f32> for Tag {
    fn from(v: f32) -> Self {
        Tag::Float(v)
    }
}
impl From<f64> for Tag {
    fn from(v: f64) -> Self {
        Tag::Double(v)
    }
}
impl From<&str> for Tag {
    fn from(v: &str) -> Self {
        Tag::String(v.to_owned())
    }
}
impl From<String> for Tag {
    fn from(v: String) -> Self {
        Tag::String(v)
    }
}
impl From<Compound> for Tag {
    fn from(v: Compound) -> Self {
        Tag::Compound(v)
    }
}
impl From<List> for Tag {
    fn from(v: List) -> Self {
        Tag::List(v)
    }
}
impl From<Vec<i32>> for Tag {
    fn from(v: Vec<i32>) -> Self {
        Tag::IntArray(v)
    }
}
impl From<Vec<i64>> for Tag {
    fn from(v: Vec<i64>) -> Self {
        Tag::LongArray(v)
    }
}

/// Convenience builder for compounds: `compound! { "a" => 1i32, "b" => "x" }`.
#[macro_export]
macro_rules! compound {
    () => { $crate::Compound::new() };
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut c = $crate::Compound::new();
        $( c.insert(::std::string::String::from($key), $crate::Tag::from($value)); )+
        c
    }};
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Format(m) => write!(f, "invalid NBT: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Format(_) => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Zlib,
}

// ---------------------------------------------------------------- reading --

struct Reader<R> {
    inner: R,
}

impl<R: Read> Reader<R> {
    fn u8(&mut self) -> Result<u8, Error> {
        let mut b = [0u8; 1];
        self.inner.read_exact(&mut b)?;
        Ok(b[0])
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut b = [0u8; N];
        self.inner.read_exact(&mut b)?;
        Ok(b)
    }
    fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    fn i64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    fn len(&mut self) -> Result<usize, Error> {
        let len = self.i32()?;
        usize::try_from(len).map_err(|_| Error::Format(format!("negative length {len}")))
    }
    fn string(&mut self) -> Result<String, Error> {
        let len = u16::from_be_bytes(self.array()?) as usize;
        let mut bytes = vec![0u8; len];
        self.inner.read_exact(&mut bytes)?;
        mutf8::decode(&bytes).ok_or_else(|| Error::Format("malformed modified UTF-8 string".into()))
    }

    fn payload(&mut self, type_id: u8, depth: usize) -> Result<Tag, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Format("maximum nesting depth exceeded".into()));
        }
        Ok(match type_id {
            TAG_BYTE => Tag::Byte(self.u8()? as i8),
            TAG_SHORT => Tag::Short(self.i16()?),
            TAG_INT => Tag::Int(self.i32()?),
            TAG_LONG => Tag::Long(self.i64()?),
            TAG_FLOAT => Tag::Float(f32::from_bits(self.i32()? as u32)),
            TAG_DOUBLE => Tag::Double(f64::from_bits(self.i64()? as u64)),
            TAG_BYTE_ARRAY => {
                let len = self.len()?;
                let mut bytes = vec![0u8; len];
                self.inner.read_exact(&mut bytes)?;
                Tag::ByteArray(bytes.into_iter().map(|b| b as i8).collect())
            }
            TAG_STRING => Tag::String(self.string()?),
            TAG_LIST => {
                let element_type = self.u8()?;
                let len = self.len()?;
                if element_type == TAG_END && len > 0 {
                    return Err(Error::Format("non-empty list of TAG_End".into()));
                }
                let mut items = Vec::with_capacity(len.min(65536));
                for _ in 0..len {
                    items.push(self.payload(element_type, depth + 1)?);
                }
                Tag::List(List {
                    element_type,
                    items,
                })
            }
            TAG_COMPOUND => {
                let mut map = Compound::new();
                loop {
                    let child_type = self.u8()?;
                    if child_type == TAG_END {
                        break;
                    }
                    let name = self.string()?;
                    let value = self.payload(child_type, depth + 1)?;
                    map.insert(name, value);
                }
                Tag::Compound(map)
            }
            TAG_INT_ARRAY => {
                let len = self.len()?;
                let mut v = Vec::with_capacity(len.min(1 << 20));
                for _ in 0..len {
                    v.push(self.i32()?);
                }
                Tag::IntArray(v)
            }
            TAG_LONG_ARRAY => {
                let len = self.len()?;
                let mut v = Vec::with_capacity(len.min(1 << 20));
                for _ in 0..len {
                    v.push(self.i64()?);
                }
                Tag::LongArray(v)
            }
            other => return Err(Error::Format(format!("unknown tag type {other}"))),
        })
    }
}

/// Reads one uncompressed named tag (normally the root compound).
pub fn read_named<R: Read>(reader: R) -> Result<(String, Tag), Error> {
    let mut r = Reader { inner: reader };
    let type_id = r.u8()?;
    if type_id == TAG_END {
        return Err(Error::Format("root tag is TAG_End".into()));
    }
    let name = r.string()?;
    let tag = r.payload(type_id, 0)?;
    Ok((name, tag))
}

/// Reads a root compound from bytes, detecting gzip / zlib / raw automatically.
pub fn from_bytes(bytes: &[u8]) -> Result<(String, Compound), Error> {
    let (name, tag) = match detect_compression(bytes) {
        Compression::Gzip => read_named(GzDecoder::new(bytes))?,
        Compression::Zlib => read_named(ZlibDecoder::new(bytes))?,
        Compression::None => read_named(bytes)?,
    };
    match tag {
        Tag::Compound(c) => Ok((name, c)),
        other => Err(Error::Format(format!(
            "root tag is type {}, expected compound",
            other.type_id()
        ))),
    }
}

pub fn detect_compression(bytes: &[u8]) -> Compression {
    match bytes {
        [0x1f, 0x8b, ..] => Compression::Gzip,
        // A zlib header has CMF 0x78 and (CMF*256 + FLG) divisible by 31. A raw
        // root compound always starts with 0x0a, so there is no ambiguity.
        [0x78, flg, ..] if (0x7800u16 | *flg as u16).is_multiple_of(31) => Compression::Zlib,
        _ => Compression::None,
    }
}

pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<(String, Compound), Error> {
    from_bytes(&std::fs::read(path)?)
}

// ---------------------------------------------------------------- writing --

struct Writer<W> {
    inner: W,
}

impl<W: Write> Writer<W> {
    fn string(&mut self, s: &str) -> Result<(), Error> {
        let bytes = mutf8::encode(s);
        let len = u16::try_from(bytes.len())
            .map_err(|_| Error::Format(format!("string too long ({} bytes)", bytes.len())))?;
        self.inner.write_all(&len.to_be_bytes())?;
        self.inner.write_all(&bytes)?;
        Ok(())
    }
    fn len(&mut self, len: usize) -> Result<(), Error> {
        let len = i32::try_from(len).map_err(|_| Error::Format("array too long".into()))?;
        self.inner.write_all(&len.to_be_bytes())?;
        Ok(())
    }

    fn payload(&mut self, tag: &Tag) -> Result<(), Error> {
        match tag {
            Tag::Byte(v) => self.inner.write_all(&[*v as u8])?,
            Tag::Short(v) => self.inner.write_all(&v.to_be_bytes())?,
            Tag::Int(v) => self.inner.write_all(&v.to_be_bytes())?,
            Tag::Long(v) => self.inner.write_all(&v.to_be_bytes())?,
            Tag::Float(v) => self.inner.write_all(&v.to_bits().to_be_bytes())?,
            Tag::Double(v) => self.inner.write_all(&v.to_bits().to_be_bytes())?,
            Tag::ByteArray(v) => {
                self.len(v.len())?;
                let bytes: Vec<u8> = v.iter().map(|b| *b as u8).collect();
                self.inner.write_all(&bytes)?;
            }
            Tag::String(s) => self.string(s)?,
            Tag::List(list) => {
                if let Some(bad) = list.items.iter().find(|t| t.type_id() != list.element_type) {
                    return Err(Error::Format(format!(
                        "list of type {} contains tag of type {}",
                        list.element_type,
                        bad.type_id()
                    )));
                }
                self.inner.write_all(&[list.element_type])?;
                self.len(list.items.len())?;
                for item in &list.items {
                    self.payload(item)?;
                }
            }
            Tag::Compound(map) => {
                for (name, value) in map {
                    self.inner.write_all(&[value.type_id()])?;
                    self.string(name)?;
                    self.payload(value)?;
                }
                self.inner.write_all(&[TAG_END])?;
            }
            Tag::IntArray(v) => {
                self.len(v.len())?;
                for i in v {
                    self.inner.write_all(&i.to_be_bytes())?;
                }
            }
            Tag::LongArray(v) => {
                self.len(v.len())?;
                for i in v {
                    self.inner.write_all(&i.to_be_bytes())?;
                }
            }
        }
        Ok(())
    }
}

/// Writes one uncompressed named tag.
pub fn write_named<W: Write>(writer: W, name: &str, tag: &Tag) -> Result<(), Error> {
    let mut w = Writer { inner: writer };
    w.inner.write_all(&[tag.type_id()])?;
    w.string(name)?;
    w.payload(tag)
}

/// Serialises a root compound to bytes with the requested compression.
pub fn to_bytes(name: &str, root: &Compound, compression: Compression) -> Result<Vec<u8>, Error> {
    // Avoid cloning the whole tree just to wrap it in Tag::Compound.
    fn write_root<W: Write>(w: W, name: &str, root: &Compound) -> Result<W, Error> {
        let mut w = Writer { inner: w };
        w.inner.write_all(&[TAG_COMPOUND])?;
        w.string(name)?;
        for (child_name, value) in root {
            w.inner.write_all(&[value.type_id()])?;
            w.string(child_name)?;
            w.payload(value)?;
        }
        w.inner.write_all(&[TAG_END])?;
        Ok(w.inner)
    }
    Ok(match compression {
        Compression::None => write_root(Vec::new(), name, root)?,
        Compression::Gzip => write_root(
            GzEncoder::new(Vec::new(), FlateLevel::default()),
            name,
            root,
        )?
        .finish()?,
        Compression::Zlib => write_root(
            ZlibEncoder::new(Vec::new(), FlateLevel::default()),
            name,
            root,
        )?
        .finish()?,
    })
}

pub fn write_file(
    path: impl AsRef<std::path::Path>,
    name: &str,
    root: &Compound,
    compression: Compression,
) -> Result<(), Error> {
    std::fs::write(path, to_bytes(name, root, compression)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Compound {
        compound! {
            "byte" => 1i8,
            "short" => -2i16,
            "int" => 123456i32,
            "long" => i64::MIN,
            "float" => 1.5f32,
            "double" => -0.25f64,
            "bytes" => Tag::ByteArray(vec![-1, 0, 1]),
            "string" => "héllo \u{0}\u{1F600}",
            "list" => List::strings(["a", "b"]),
            "empty" => List::empty(TAG_COMPOUND),
            "nested" => compound! { "x" => 1i32 },
            "ints" => vec![1i32, -1],
            "longs" => vec![i64::MAX, 0],
        }
    }

    #[test]
    fn round_trips_every_compression() {
        let root = sample();
        for compression in [Compression::None, Compression::Gzip, Compression::Zlib] {
            let bytes = to_bytes("root", &root, compression).unwrap();
            assert_eq!(detect_compression(&bytes), compression);
            let (name, back) = from_bytes(&bytes).unwrap();
            assert_eq!(name, "root");
            assert_eq!(back, root);
        }
    }

    #[test]
    fn encodes_known_bytes() {
        let bytes = to_bytes("", &compound! { "a" => 1i8 }, Compression::None).unwrap();
        assert_eq!(bytes, [10, 0, 0, 1, 0, 1, b'a', 1, 0]);
    }

    #[test]
    fn rejects_mixed_lists() {
        assert!(List::of(vec![Tag::Int(1), Tag::Byte(1)]).is_err());
    }

    #[test]
    fn rejects_truncated_input() {
        let bytes = to_bytes("", &sample(), Compression::None).unwrap();
        assert!(from_bytes(&bytes[..bytes.len() - 3]).is_err());
    }
}
