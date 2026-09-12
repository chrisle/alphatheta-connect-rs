# alphatheta-connect-rs

Rust port of [chrisle/alphatheta-connect](https://github.com/chrisle/alphatheta-connect)
(TypeScript). The port follows upstream: `UPSTREAM_COMMIT` holds the upstream
`main` commit it matches, and `.github/workflows/sync-upstream.yml` ports
anything newer with a Claude agent and opens a pull request.

## Layout

`src/` mirrors upstream `src/` module for module (`src/status/utils.ts` →
`src/status/utils.rs`, `src/remotedb/message/item.ts` →
`src/remotedb/message/item.rs`). Two upstream dependencies are inlined:
`onelibrary-connect` under `src/localdb/onelibrary/`, `metadata-connect`
under `src/metadata/`. Upstream's Kaitai `.ksy` files are hand-written
parsers: `src/localdb/rekordbox/pdb.rs` and `src/localdb/rekordbox/anlz.rs`.

## Conventions

- Events: one `Emitter<T>` per event (`subscribe()`, `on(cb)`, `once(pred)`),
  not a stringly-typed event emitter.
- I/O is `tokio`; sockets are `UdpFeed`s that re-broadcast datagrams so
  several services can listen on one port.
- Errors go through `crate::Error` (`thiserror`); `Option` for the upstream
  `null`s; enums carry an `Other(u8)` variant where the wire may hold values
  upstream does not name.
- Doc comments are rewritten from the upstream JSDoc, not pasted.
- Packet and file byte layouts are the contract: copy offsets exactly.

## Checks

```
cargo fmt --all
cargo build --all-features
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

The `passive` feature links libpcap; `cli` builds the demo binary. SQLCipher
is built from source (needs a C toolchain and perl).

## Do not

- Commit `.claude/` (git-ignored on purpose: the porting agent and skill are
  installed on the runner host by `scripts/install-agent.sh`).
- Edit `.github/` or `scripts/` from the porting agent.
