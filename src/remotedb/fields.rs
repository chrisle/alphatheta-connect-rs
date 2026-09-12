//! Remote database wire fields.
//!
//! Every value in a remotedb message is a field: a leading type byte followed
//! by the payload (with a length header for the variable-size kinds).

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::{Error, Result};

/// Field type is a leading byte that indicates what the field is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FieldType {
    UInt8 = 0x0f,
    UInt16 = 0x10,
    UInt32 = 0x11,
    Binary = 0x14,
    String = 0x26,
}

impl FieldType {
    pub const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0x0f => FieldType::UInt8,
            0x10 => FieldType::UInt16,
            0x11 => FieldType::UInt32,
            0x14 => FieldType::Binary,
            0x26 => FieldType::String,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            FieldType::UInt8 => "UInt8",
            FieldType::UInt16 => "UInt16",
            FieldType::UInt32 => "UInt32",
            FieldType::Binary => "Binary",
            FieldType::String => "String",
        }
    }
}

/// A decoded field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    /// Binary data.
    Binary(Vec<u8>),
    /// A null-terminated big endian UTF-16 string, decoded.
    String(String),
}

impl Field {
    pub const fn field_type(&self) -> FieldType {
        match self {
            Field::UInt8(_) => FieldType::UInt8,
            Field::UInt16(_) => FieldType::UInt16,
            Field::UInt32(_) => FieldType::UInt32,
            Field::Binary(_) => FieldType::Binary,
            Field::String(_) => FieldType::String,
        }
    }

    /// A `UInt32` field packing four bytes big-endian (upstream's
    /// `new UInt32(Buffer.of(a, b, c, d))`).
    pub const fn u32_from_bytes(bytes: [u8; 4]) -> Field {
        Field::UInt32(u32::from_be_bytes(bytes))
    }

    /// The numeric value of a number field, `None` for the other kinds.
    pub fn as_number(&self) -> Option<u32> {
        match self {
            Field::UInt8(v) => Some(u32::from(*v)),
            Field::UInt16(v) => Some(u32::from(*v)),
            Field::UInt32(v) => Some(*v),
            _ => None,
        }
    }

    /// The string value, `None` for the other kinds.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Field::String(s) => Some(s),
            _ => None,
        }
    }

    /// The binary payload, `None` for the other kinds.
    pub fn as_binary(&self) -> Option<&[u8]> {
        match self {
            Field::Binary(b) => Some(b),
            _ => None,
        }
    }

    /// The raw field data, without the type header.
    pub fn data(&self) -> Vec<u8> {
        match self {
            Field::UInt8(v) => vec![*v],
            Field::UInt16(v) => v.to_be_bytes().to_vec(),
            Field::UInt32(v) => v.to_be_bytes().to_vec(),
            Field::Binary(b) => b.clone(),
            Field::String(s) => encode_utf16be_nul(s),
        }
    }

    /// Coerce the field into bytes. This differs from [`data`](Self::data)
    /// in that it includes the field type header (and length header for the
    /// variable-size kinds).
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Field::UInt8(_) | Field::UInt16(_) | Field::UInt32(_) => {
                let mut out = vec![self.field_type() as u8];
                out.extend(self.data());
                out
            }
            Field::Binary(b) => make_variable_buffer(FieldType::Binary, b, b.len() as u32),
            Field::String(s) => {
                let data = encode_utf16be_nul(s);
                let len = (data.len() / 2) as u32;
                make_variable_buffer(FieldType::String, &data, len)
            }
        }
    }
}

fn encode_utf16be_nul(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2 + 2);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

fn decode_utf16be_nul(data: &[u8]) -> String {
    let units: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    // Slice off the trailing null
    let end = units.len().saturating_sub(1);
    String::from_utf16_lossy(&units[..end])
}

fn make_variable_buffer(field_type: FieldType, field_data: &[u8], length_header: u32) -> Vec<u8> {
    // Add 4 bytes for length header and 1 byte for type header.
    let mut data = Vec::with_capacity(field_data.len() + 5);
    data.push(field_type as u8);
    data.extend_from_slice(&length_header.to_be_bytes());
    data.extend_from_slice(field_data);
    data
}

/// Read a single field from a stream, requiring it to be of type `expect`.
pub async fn read_field<R: AsyncRead + Unpin>(stream: &mut R, expect: FieldType) -> Result<Field> {
    let type_byte = stream.read_u8().await?;
    let field_type =
        FieldType::from_u8(type_byte).ok_or_else(|| Error::RemoteDb(format!("Unknown field type 0x{type_byte:02x}")))?;

    if field_type != expect {
        return Err(Error::RemoteDb(format!("Expected {} but got {}", expect.name(), field_type.name())));
    }

    match field_type {
        FieldType::UInt8 => Ok(Field::UInt8(stream.read_u8().await?)),
        FieldType::UInt16 => Ok(Field::UInt16(stream.read_u16().await?)),
        FieldType::UInt32 => Ok(Field::UInt32(stream.read_u32().await?)),
        FieldType::Binary => {
            // Read the field length as a UInt32 when we do not know the field
            // length from the type
            let len = stream.read_u32().await? as usize;
            let mut data = vec![0u8; len];
            if len > 0 {
                stream.read_exact(&mut data).await?;
            }
            Ok(Field::Binary(data))
        }
        FieldType::String => {
            // A UTF-16 string takes 2 bytes per character.
            let len = stream.read_u32().await? as usize * 2;
            let mut data = vec![0u8; len];
            if len > 0 {
                stream.read_exact(&mut data).await?;
            }
            Ok(Field::String(decode_utf16be_nul(&data)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_fields_encode() {
        let f = Field::UInt8(5);
        assert_eq!(f.data(), vec![0x05]);
        assert_eq!(f.to_bytes(), vec![0x0f, 0x05]);
        let f = Field::UInt16(5);
        assert_eq!(f.data(), vec![0x00, 0x05]);
        assert_eq!(f.to_bytes(), vec![0x10, 0x00, 0x05]);
        let f = Field::UInt32(5);
        assert_eq!(f.data(), vec![0, 0, 0, 5]);
        assert_eq!(f.to_bytes(), vec![0x11, 0, 0, 0, 5]);
    }

    #[test]
    fn string_field_encodes_utf16be_with_nul_and_char_length() {
        let f = Field::String("ab".into());
        assert_eq!(f.data(), vec![0, b'a', 0, b'b', 0, 0]);
        let bytes = f.to_bytes();
        assert_eq!(bytes[0], 0x26);
        assert_eq!(&bytes[1..5], &[0, 0, 0, 3]);
        assert_eq!(bytes.len(), 5 + 6);
    }

    #[test]
    fn binary_field_encodes_with_byte_length() {
        let f = Field::Binary(vec![1, 2, 3]);
        let bytes = f.to_bytes();
        assert_eq!(bytes, vec![0x14, 0, 0, 0, 3, 1, 2, 3]);
        assert_eq!(Field::Binary(vec![]).to_bytes(), vec![0x14, 0, 0, 0, 0]);
    }

    #[tokio::test]
    async fn reads_fields_back() {
        let mut stream: Vec<u8> = Vec::new();
        stream.extend(Field::UInt32(7).to_bytes());
        stream.extend(Field::String("hi".into()).to_bytes());
        stream.extend(Field::Binary(vec![9]).to_bytes());
        stream.extend(Field::UInt8(1).to_bytes());
        let mut cursor = std::io::Cursor::new(stream);
        assert_eq!(read_field(&mut cursor, FieldType::UInt32).await.unwrap(), Field::UInt32(7));
        assert_eq!(read_field(&mut cursor, FieldType::String).await.unwrap(), Field::String("hi".into()));
        assert_eq!(read_field(&mut cursor, FieldType::Binary).await.unwrap(), Field::Binary(vec![9]));
        assert!(read_field(&mut cursor, FieldType::UInt32).await.is_err());
    }
}
