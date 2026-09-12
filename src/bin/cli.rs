//! A small demonstration CLI, the equivalent of upstream's `src/cli`: bring
//! the network online, connect, print the track loaded on each player and
//! copy its file from the player's media into the current directory.

use std::collections::HashMap;
use std::sync::Arc;

use alphatheta_connect::db::{get_file, get_metadata};
use alphatheta_connect::{bring_online, MixstatusProcessor, NetworkConfig, TracingLogger};

#[tokio::main]
async fn main() -> alphatheta_connect::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("alphatheta_connect=info".parse().unwrap()),
        )
        .init();

    eprintln!("Bringing up prolink network");
    let network = bring_online(Some(NetworkConfig { logger: Some(Arc::new(TracingLogger)), ..Default::default() })).await?;
    eprintln!("Network online, preparing to connect");

    let _new_device = network.device_manager().on_connected(|d| {
        eprintln!("New device: {} [id: {}]", d.name, d.id);
    });

    eprintln!("Autoconfiguring network.. waiting for devices");
    network.autoconfig_from_peers().await?;
    eprintln!("Autoconfigure successful!");

    eprintln!("Connecting to network!");
    network.connect().await?;

    if !network.is_connected() {
        eprintln!("Failed to connect to the network");
        return Ok(());
    }

    eprintln!("Network connected! Network services initialized");

    let status = network.status_emitter().expect("connected").clone();
    let db = network.db().expect("connected");

    let processor = MixstatusProcessor::new(None);
    let feed = processor.clone();
    let _mix = status.on_status(move |s| feed.handle_state(s));
    let _now_playing = processor.on_now_playing(|s| eprintln!("Now playing on player {}: track {}", s.device_id, s.track_id));

    let mut last_tid: HashMap<u8, u32> = HashMap::new();
    let mut rx = status.subscribe_status();

    while let Ok(state) = rx.recv().await {
        if last_tid.get(&state.device_id) == Some(&state.track_id) {
            continue;
        }
        last_tid.insert(state.device_id, state.track_id);

        let track = db
            .get_metadata(get_metadata::Options {
                device_id: state.track_device_id,
                track_slot: state.track_slot,
                track_type: state.track_type,
                track_id: state.track_id,
                track_bpm: state.track_bpm,
            })
            .await?;

        let Some(track) = track else {
            eprintln!("no track");
            continue;
        };

        println!("{} {}", state.track_id, track.title);

        // Download the file from ProDJ-Link.
        let file_name = track.file_name.clone();
        let buf = db
            .get_file(get_file::Options {
                device_id: state.track_device_id,
                track_slot: state.track_slot,
                track_type: state.track_type,
                track,
                logger: None,
            })
            .await?;
        if let Some(buf) = buf {
            if !file_name.is_empty() {
                tokio::fs::write(&file_name, buf).await?;
                println!("Copied {file_name}");
            }
        }
    }

    Ok(())
}
