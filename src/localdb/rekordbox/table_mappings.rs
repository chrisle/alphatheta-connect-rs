//! Maps rekordbox pdb table types to orm tables.

use crate::localdb::orm::Table;
use crate::localdb::rekordbox::pdb::PageType;

/// The orm table a pdb table type hydrates into, `None` for tables that are
/// not hydrated.
pub fn pdb_table(page_type: PageType) -> Option<Table> {
    Some(match page_type {
        PageType::Tracks => Table::Track,
        PageType::Artists => Table::Artist,
        PageType::Genres => Table::Genre,
        PageType::Albums => Table::Album,
        PageType::Labels => Table::Label,
        PageType::Colors => Table::Color,
        PageType::Keys => Table::Key,
        PageType::Artwork => Table::Artwork,
        PageType::PlaylistTree => Table::Playlist,
        PageType::PlaylistEntries => Table::PlaylistEntry,
        // TODO: Register PageType::History
        _ => return None,
    })
}
