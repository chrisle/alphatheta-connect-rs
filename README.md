# alphatheta-connect (Rust)

AlphaTheta's PRO DJ LINK protocol, unlocked: consume CDJ state and retrieve
complete track metadata.

This crate is a Rust port of the TypeScript library
[alphatheta-connect](https://github.com/chrisle/alphatheta-connect), which
powers [Now Playing](https://nowplayingapp.com/). The module layout mirrors
the upstream `src/` tree file by file, and the port is kept in step with
upstream automatically (see [Staying in sync](#staying-in-sync)).

Alternative implementations of the Prolink protocol:
[Java](https://github.com/Deep-Symmetry/beat-link),
[Go](https://github.com/evanpurkhiser/prolink-go).

## Features

- **Streaming service detection** — detect tracks loaded from Beatport,
  Streaming Direct Play, TIDAL and Apple Music via the `MediaSlot` enum.
- **All-in-one units (Opus Quad, XDJ-RX3/RX2/RX, XDJ-XZ)** — passive
  monitoring via pcap packet capture where a virtual CDJ cannot join
  (`passive` feature). See [docs/ALL_IN_ONE_UNITS.md](docs/ALL_IN_ONE_UNITS.md).
- **Pioneer Stagehand connection mode** — join as a virtual Stagehand iPad
  device for mixer fader/EQ/VU telemetry and remote control of CDJs. See
  [docs/STAGEHAND.md](docs/STAGEHAND.md).
- **OneLibrary support** — rekordbox 7's `exportLibrary.db` (SQLCipher) with
  tracks, playlists, cues, hot cue banks, MyTags and history.
- **6-channel on-air support** — CDJ-3000 / DJM-V10 rigs. See
  [docs/ON_AIR_CHANNELS.md](docs/ON_AIR_CHANNELS.md).
- **Artwork and metadata extraction over NFS** — read only file headers off
  the connected media (MP3, M4A, FLAC, AIFF).
- **Extended ANLZ support** — beat grids, cues with colours and comments
  (PCO2), song structure (PSSI), every waveform format (PWAV…PWV7), vocal
  detection (PWVC). See [docs/EXTENDED_ANLZ.md](docs/EXTENDED_ANLZ.md).
- **CDJ-3000 absolute position tracking** (30 ms updates) and the full
  startup handshake. See [docs/ABSOLUTE_POSITION.md](docs/ABSOLUTE_POSITION.md)
  and [docs/FULL_STARTUP.md](docs/FULL_STARTUP.md).
- **Mix status** — "now playing" / "stopped" / set lifecycle events derived
  from player state.

## Usage

```toml
[dependencies]
alphatheta-connect = "0.25"
tokio = { version = "1", features = ["full"] }
```

```rust
use alphatheta_connect::{bring_online, db::get_metadata};

#[tokio::main]
async fn main() -> alphatheta_connect::Result<()> {
    // Bring the prolink network online. This binds UDP 50000-50002 and will
    // FAIL if rekordbox, or a second instance of this library, is running on
    // the same machine.
    let network = bring_online(None).await?;

    // React to devices appearing on the network.
    let _listener = network.device_manager().on_connected(|device| {
        println!("New device on network: {} [id {}]", device.name, device.id);
    });

    // Wait for a peer to pick the interface, then join as a virtual CDJ.
    // Device id 7 is used by default: outside the player range, so it can
    // never knock a real CDJ off the network.
    network.autoconfig_from_peers().await?;
    network.connect().await?;

    let status = network.status_emitter().expect("connected").clone();
    let db = network.db().expect("connected");

    let mut rx = status.subscribe_status();
    while let Ok(state) = rx.recv().await {
        let track = db
            .get_metadata(get_metadata::Options {
                device_id: state.track_device_id,
                track_slot: state.track_slot,
                track_type: state.track_type,
                track_id: state.track_id,
                track_bpm: state.track_bpm,
            })
            .await?;
        if let Some(track) = track {
            println!("player {} loaded {}", state.device_id, track.title);
        }
    }
    Ok(())
}
```

Events are `Emitter<T>` values: `subscribe()` for a channel, `on(callback)`
for a background callback, `once(pred)` to await one event.

### Features

| Feature   | Adds                                                                                  |
| --------- | ------------------------------------------------------------------------------------- |
| `passive` | pcap capture for `bring_online_passive` (links libpcap: `libpcap-dev`, Npcap)         |
| `cli`     | the `alphatheta-connect` demo binary                                                  |

OneLibrary support builds SQLCipher from source with a vendored OpenSSL, so a C
toolchain and `perl` are needed to build the crate.

## Staying in sync

`UPSTREAM_COMMIT` records the upstream `main` commit this port matches. The
`sync-upstream` workflow (on a schedule, or immediately on a
`repository_dispatch` from upstream) ports any newer upstream commits with a
Claude agent, gates the result with `cargo build` / `test` / `clippy`, and
opens a pull request. The agent and its skill are not part of this repository;
they are installed on the runner host with `scripts/install-agent.sh`.

## Thanks To

- [@evanpurkhiser](https://github.com/evanpurkhiser) — original author of
  alphatheta-connect (formerly prolink-connect) and
  [Prolink Tools](https://prolink.tools/)
- [@brunchboy](https://github.com/brunchboy) — for
  [dysentery](https://github.com/brunchboy/dysentery) and
  [beat-link](https://github.com/Deep-Symmetry/beat-link)
- [Deep Symmetry](https://github.com/Deep-Symmetry) — for
  [crate-digger](https://github.com/Deep-Symmetry/crate-digger) and the Pro DJ
  Link protocol documentation
- [@henrybetts](https://github.com/henrybetts) and
  [@flesniak](https://github.com/flesniak) — for reverse-engineering the
  rekordbox database format

## License

MIT
