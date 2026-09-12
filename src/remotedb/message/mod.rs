//! Remote database messages: a set of fields sequenced into a known message
//! format.

pub mod item;
pub mod response;
pub mod types;

use tokio::io::AsyncRead;

use crate::remotedb::constants::REMOTEDB_MAGIC;
use crate::remotedb::fields::{read_field, Field, FieldType};
use crate::{Error, Result};

pub use item::{fields_to_item, Item, ItemType};
pub use response::{response_transform, ResponseData};
pub use types::response as response_type;
pub use types::{control_request, data_request, is_response, menu_request, message_name, MessageType};

/// Argument types are used in argument list fields. This is essentially
/// duplicating the field type, but has different values for whatever reason.
///
/// There do not appear to be argument types for UInt8 and UInt16. At least,
/// no messages include these field types as arguments as far as we know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ArgumentType {
    String = 0x02,
    Binary = 0x03,
    UInt32 = 0x06,
}

impl ArgumentType {
    const fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0x02 => ArgumentType::String,
            0x03 => ArgumentType::Binary,
            0x06 => ArgumentType::UInt32,
            _ => return None,
        })
    }

    const fn field_type(self) -> FieldType {
        match self {
            ArgumentType::String => FieldType::String,
            ArgumentType::Binary => FieldType::Binary,
            ArgumentType::UInt32 => FieldType::UInt32,
        }
    }

    const fn for_field(t: FieldType) -> u8 {
        match t {
            FieldType::UInt32 => ArgumentType::UInt32 as u8,
            FieldType::String => ArgumentType::String as u8,
            FieldType::Binary => ArgumentType::Binary as u8,
            // The following two field types do not have associated argument
            // types (see the note above).
            FieldType::UInt8 | FieldType::UInt16 => 0x00,
        }
    }
}

/// The message argument list always contains 12 slots.
const ARG_COUNT: usize = 12;

/// Representation of a set of fields sequenced into a known message format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The transaction ID is used to associate responses to their requests.
    pub transaction_id: Option<u32>,
    pub message_type: MessageType,
    pub args: Vec<Field>,
}

impl Message {
    pub fn new(message_type: MessageType, args: Vec<Field>) -> Self {
        Self { transaction_id: None, message_type, args }
    }

    pub fn with_transaction(transaction_id: u32, message_type: MessageType, args: Vec<Field>) -> Self {
        Self { transaction_id: Some(transaction_id), message_type, args }
    }

    /// Read a single message from a readable stream, requiring it to be of
    /// type `expect`.
    pub async fn from_stream<R: AsyncRead + Unpin>(stream: &mut R, expect: MessageType) -> Result<Message> {
        // 01. Read magic bytes
        let magic = read_field(stream, FieldType::UInt32).await?;
        if magic.as_number() != Some(REMOTEDB_MAGIC) {
            return Err(Error::RemoteDb("Did not receive expected magic value. Corrupt message".into()));
        }

        // 02. Read transaction ID
        let tx_id = read_field(stream, FieldType::UInt32).await?.as_number().unwrap_or(0);

        // 03. Read message type
        let message_type = read_field(stream, FieldType::UInt16).await?.as_number().unwrap_or(0) as u16;

        // 04. Read argument count
        let arg_count = read_field(stream, FieldType::UInt8).await?.as_number().unwrap_or(0) as usize;

        // 05. Read argument list
        let arg_list = match read_field(stream, FieldType::Binary).await? {
            Field::Binary(b) => b,
            _ => unreachable!(),
        };

        // 06. Read all argument fields in
        let mut args: Vec<Field> = Vec::with_capacity(arg_count);
        for i in 0..arg_count {
            let arg_type = arg_list
                .get(i)
                .copied()
                .and_then(ArgumentType::from_u8)
                .ok_or_else(|| Error::RemoteDb(format!("unknown argument type in slot {i}")))?;

            // XXX: There is a small quirk in a few message response types that
            //      send binary data, but if the binary data is empty the field
            //      will not be sent.
            if arg_type == ArgumentType::Binary && i > 0 && args[i - 1].as_number() == Some(0) {
                args.push(Field::Binary(Vec::new()));
                continue;
            }

            args.push(read_field(stream, arg_type.field_type()).await?);
        }

        if message_type != expect {
            return Err(Error::RemoteDb(format!("Expected message type 0x{expect:x}, got 0x{message_type:x}")));
        }

        Ok(Message { transaction_id: Some(tx_id), message_type, args })
    }

    /// The byte serialization of the message.
    pub fn to_bytes(&self) -> Vec<u8> {
        // Determine the argument list from the list of fields
        let mut arg_list = vec![0u8; ARG_COUNT];
        for (slot, arg) in arg_list.iter_mut().zip(&self.args) {
            *slot = ArgumentType::for_field(arg.field_type());
        }

        // XXX: Following the parsing quirk for messages that contain binary
        //      data but are _empty_, we check for binary fields with UInt32
        //      fields before with the value of 0 (indicating "an empty binary
        //      field").
        let args: Vec<&Field> = self
            .args
            .iter()
            .enumerate()
            .filter(|(i, arg)| {
                let is_empty_buffer =
                    arg.field_type() == FieldType::Binary && *i != 0 && matches!(self.args[i - 1], Field::UInt32(0));
                !is_empty_buffer
            })
            .map(|(_, arg)| arg)
            .collect();

        let mut out = Vec::new();
        out.extend(Field::UInt32(REMOTEDB_MAGIC).to_bytes());
        out.extend(Field::UInt32(self.transaction_id.unwrap_or(0)).to_bytes());
        out.extend(Field::UInt16(self.message_type).to_bytes());
        out.extend(Field::UInt8(self.args.len() as u8).to_bytes());
        out.extend(Field::Binary(arg_list).to_bytes());
        for arg in args {
            out.extend(arg.to_bytes());
        }
        out
    }

    /// The structured representation of the message. Currently only supports
    /// representing response messages.
    pub fn data(&self) -> Result<ResponseData> {
        if !is_response(self.message_type) {
            return Err(Error::RemoteDb("Representation of non-responses is not currently supported".into()));
        }
        response_transform(self.message_type, &self.args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_a_message() {
        let msg = Message::with_transaction(
            7,
            response_type::SUCCESS,
            vec![Field::UInt32(1), Field::UInt32(42), Field::String("x".into())],
        );
        let bytes = msg.to_bytes();
        let mut cursor = std::io::Cursor::new(bytes);
        let back = Message::from_stream(&mut cursor, response_type::SUCCESS).await.unwrap();
        assert_eq!(back, msg);
        assert_eq!(back.data().unwrap(), ResponseData::Success { items_available: 42 });
    }

    #[tokio::test]
    async fn empty_binary_after_zero_is_elided_and_restored() {
        let msg = Message::with_transaction(
            1,
            response_type::ARTWORK,
            vec![Field::UInt32(1), Field::UInt32(2), Field::UInt32(0), Field::Binary(vec![])],
        );
        let bytes = msg.to_bytes();
        // header (5+5+3+2+17 = 32) + three UInt32 (15) and no binary field
        assert_eq!(bytes.len(), 32 + 15);
        let mut cursor = std::io::Cursor::new(bytes);
        let back = Message::from_stream(&mut cursor, response_type::ARTWORK).await.unwrap();
        assert_eq!(back.args, msg.args);
    }

    #[tokio::test]
    async fn wrong_type_is_an_error() {
        let msg = Message::with_transaction(1, response_type::MENU_HEADER, vec![]);
        let mut cursor = std::io::Cursor::new(msg.to_bytes());
        assert!(Message::from_stream(&mut cursor, response_type::SUCCESS).await.is_err());
    }
}
