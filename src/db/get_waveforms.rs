//! Waveform lookups.

use crate::db::get_artwork_thumbnail::Options;
use crate::db::utils::anlz_loader;
use crate::localdb::rekordbox::{load_anlz, AnlzKind};
use crate::localdb::LocalDatabase;
use crate::remotedb::{MenuTarget, QueryDescriptor, RemoteDatabase};
use crate::types::{Device, TrackType, Waveforms};
use crate::{Error, Result};

pub async fn via_remote(remote: &RemoteDatabase, opts: &Options) -> Result<Option<Waveforms>> {
    let Some(conn) = remote.get(opts.device_id).await? else {
        return Ok(None);
    };

    let descriptor = QueryDescriptor { track_slot: opts.track_slot, track_type: opts.track_type, menu_target: MenuTarget::Main };

    let waveform_hd = conn.get_waveform_hd(&descriptor, opts.track.id).await?;

    if opts.track_type != TrackType::Streaming {
        return Ok(Some(Waveforms { waveform_hd, ..Default::default() }));
    }

    // Streaming tracks (e.g. Beatport LINK) have no local ANLZ file but the
    // CDJ serves waveform preview and detailed via remotedb.
    let waveform_preview = conn.get_waveform_preview(&descriptor, opts.track.id).await?;
    let waveform_detailed = conn.get_waveform_detailed(&descriptor, opts.track.id).await?;

    Ok(Some(Waveforms {
        waveform_hd,
        waveform_color_preview: None,
        waveform_preview: Some(waveform_preview),
        waveform_detailed: Some(waveform_detailed),
    }))
}

pub async fn via_local(local: &LocalDatabase, device: &Device, opts: &Options) -> Result<Option<Waveforms>> {
    if !opts.track_slot.is_database_slot() {
        return Err(Error::State("Expected USB or SD slot for local database query".into()));
    }

    if local.get(opts.device_id, opts.track_slot).await?.is_none() {
        return Ok(None);
    }

    let Some(analyze_path) = opts.track.analyze_path.as_deref() else {
        return Ok(Some(Waveforms::default()));
    };

    let resolver = anlz_loader(device, opts.track_slot);
    let anlz = load_anlz(analyze_path, AnlzKind::Ext, &resolver).await?;

    Ok(Some(Waveforms {
        waveform_hd: anlz.ext.waveform_hd.unwrap_or_default(),
        waveform_color_preview: anlz.ext.waveform_color_preview,
        waveform_preview: None,
        waveform_detailed: None,
    }))
}
