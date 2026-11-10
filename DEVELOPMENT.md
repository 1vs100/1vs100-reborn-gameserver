# Development

Usage is covered in [README.md](README.md) and deployment in [DEPLOYMENT.md](DEPLOYMENT.md).

## Checks

```
cargo fmt && cargo clippy --locked --all-targets && cargo test --locked
```

The storage tests also run against [PostgreSQL](https://www.postgresql.org) when `GAMESERVER_TEST_DATABASE_URL` names a database. Each test creates its own schema there and leaves it behind, so use a database you keep for tests. CI does this.

```
GAMESERVER_TEST_DATABASE_URL=postgres://onevs100:onevs100@localhost/onevs100_test cargo test --locked storage
```

`keys/grid_lan_private.pem` is committed and compiled in on purpose. It is not a secret, and it must not be rotated (`keys/README.md`).

The frontend API client is generated from `src/admin.rs`. `pnpm api:generate` runs `src/bin/openapi_dump.rs` and then `orval`. `frontend/openapi.json` and `frontend/src/api/` are generated and git-ignored.

`pnpm dev` and `pnpm preview` proxy `/admin` to `ADMIN_URL` (default `http://127.0.0.1:8081`).

## Recording a game for regression tests

Set `GAMESERVER_RECORD_SBG=<file>` and play a game. Every SBG frame in and out is written with its time, connection, console address and XUID. Commit the file as `fixtures/sessions/<date>-<build>-<what>.sbg`. `cargo test` (the `recorded_sessions` test) then fails whenever a later change answers any of its requests differently.

The replay runs on a virtual clock that starts at the recording's start time and jumps from one recorded frame to the next, so a full game replays in about a second. `./target/release/sbg_replay <file>` replays one file and lists every differing reply.

To read a recording, run `sbg_replay --list <file>`. It prints every request and reply as text. Narrow it with `--conn`, `--xuid`, `--peer` or `--key`, for example `--xuid e0000601484d4b37 --key ResetID`.

All recorded sessions must share the service host and server-flow setting, since they replay in one process. Recordings made with a show schedule are refused, because the schedule itself is not recorded.
