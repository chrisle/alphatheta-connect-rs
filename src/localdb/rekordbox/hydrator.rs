//! Translating rekordbox database (pdb) files into the entity types used in
//! this library.

use crate::localdb::orm::MetadataORM;
use crate::localdb::rekordbox::entity_creators::*;
use crate::localdb::rekordbox::pdb::{RekordboxPdb, Row};
use crate::localdb::rekordbox::table_mappings::pdb_table;
use crate::localdb::rekordbox::types::HydrationProgress;
use crate::Result;

/// Called as hydration progresses.
pub type ProgressCallback<'a> = dyn FnMut(HydrationProgress) + Send + 'a;

/// Hydrates a [`MetadataORM`] from pdb rows.
pub struct RekordboxHydrator<'a, 'p> {
    orm: &'a MetadataORM,
    on_progress: Option<&'a mut ProgressCallback<'p>>,
}

impl<'a, 'p> RekordboxHydrator<'a, 'p> {
    pub fn new(orm: &'a MetadataORM, on_progress: Option<&'a mut ProgressCallback<'p>>) -> Self {
        Self { orm, on_progress }
    }

    /// Extract entries from a rekordbox pdb file and hydrate the database
    /// with entities derived from the rekordbox entries.
    pub async fn hydrate_from_pdb(&mut self, pdb_data: &[u8]) -> Result<()> {
        let db = RekordboxPdb::parse(pdb_data)?;

        for table in &db.tables {
            let Some(orm_table) = pdb_table(table.page_type) else {
                continue;
            };
            let rows = db.table_rows(table)?;
            self.hydrate_rows(orm_table.name(), rows).await;
        }

        Ok(())
    }

    async fn hydrate_rows(&mut self, table_name: &str, rows: Vec<Row>) {
        let total = rows.len();
        let mut saved = 0usize;

        for row in rows {
            match row {
                Row::Track(r) => self.orm.insert_track(create_track(&r)),
                Row::Artist(r) => self.orm.insert_artist(create_artist(&r)),
                Row::Genre(r) => self.orm.insert_genre(create_genre(&r)),
                Row::Album(r) => self.orm.insert_album(create_album(&r)),
                Row::Label(r) => self.orm.insert_label(create_label(&r)),
                Row::Color(r) => self.orm.insert_color(create_color(&r)),
                Row::Key(r) => self.orm.insert_key(create_key(&r)),
                Row::Artwork(r) => self.orm.insert_artwork(create_artwork_entry(&r)),
                Row::PlaylistTree(r) => self.orm.insert_playlist(create_playlist(&r)),
                Row::PlaylistEntry(r) => self.orm.insert_playlist_entry(create_playlist_entry(&r)),
            }
            saved += 1;

            // Report progress and yield every 100 rows to keep the runtime
            // responsive.
            if saved % 100 == 0 || saved == total {
                if let Some(cb) = self.on_progress.as_deref_mut() {
                    cb(HydrationProgress { complete: saved, table: table_name.to_string(), total });
                }
                tokio::task::yield_now().await;
            }
        }
    }
}
