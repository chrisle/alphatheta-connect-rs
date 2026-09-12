//! Helpers for menu rendering.

use crate::remotedb::fields::Field;
use crate::remotedb::message::item::{Item, ItemType};
use crate::remotedb::message::{control_request, response_type as response, Message, ResponseData};
use crate::remotedb::{Connection, LookupDescriptor};
use crate::{Error, Result};

/// Specifies the number of items we should request at a time in menu render
/// requests.
const LIMIT: u32 = 64;

/// The `UInt32` field that carries the host device id, menu target, track
/// slot and track type.
pub fn field_from_descriptor(d: &LookupDescriptor) -> Field {
    Field::u32_from_bytes([d.host_device.id, d.menu_target.as_u8(), d.track_slot.as_u8(), d.track_type.as_u8()])
}

pub fn make_render_message(descriptor: &LookupDescriptor, offset: u32, count: u32, total: u32) -> Message {
    Message::new(
        control_request::RENDER_MENU,
        vec![
            field_from_descriptor(descriptor),
            Field::UInt32(offset),
            Field::UInt32(count),
            Field::UInt32(0),
            Field::UInt32(total),
            Field::UInt32(0x0c),
        ],
    )
}

/// Page through menu results after a successful lookup request, collecting
/// every item.
pub async fn render_items(conn: &Connection, descriptor: &LookupDescriptor, total: u32) -> Result<Vec<Item>> {
    let mut items = Vec::with_capacity(total as usize);
    let mut items_read = 0u32;

    while items_read < total {
        // Request another page of items
        if items_read % LIMIT == 0 {
            // XXX: itemsRead + count should NOT exceed the total. A larger
            // value will push the offset back to accommodate for the extra
            // items, ensuring we always receive count items.
            let count = LIMIT.min(total - items_read);
            let message = make_render_message(descriptor, items_read, count, total);

            conn.write_message(message).await?;
            conn.read_message(response::MENU_HEADER).await?;
        }

        // Read each item. Ignoring headers and footers, we will determine when
        // to stop by counting the items read until we reach the total items.
        let resp = conn.read_message(response::MENU_ITEM).await?;
        match resp.data()? {
            ResponseData::MenuItem(item) => items.push(item),
            other => return Err(Error::RemoteDb(format!("expected a menu item, got {other:?}"))),
        }
        items_read += 1;

        // When we've reached the end of a page we must read the footer
        if items_read % LIMIT == 0 || items_read == total {
            conn.read_message(response::MENU_FOOTER).await?;
        }
    }

    Ok(items)
}

/// Locate the (last) color item in an item list.
pub fn find_color(items: &[Item]) -> Option<&Item> {
    items.iter().rfind(|i| i.item_type.is_color())
}

/// The first item of a given type.
pub fn find_type(items: &[Item], t: ItemType) -> Option<&Item> {
    items.iter().find(|i| i.item_type == t)
}
