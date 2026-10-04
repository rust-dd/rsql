# RSQL

A desktop PostgreSQL client for macOS, Windows and Linux, built with Tauri, React and Rust. Free and open source under the MIT license. It needs no account and collects no telemetry.

![RSQL screenshot](docs/rsql.png)

## Install

Download the installer for your platform from the [latest release](https://github.com/rust-dd/rsql/releases/latest):

- macOS: `rsql_<version>_aarch64.dmg` (Apple Silicon) or `rsql_<version>_x64.dmg` (Intel)
- Windows (x64): `rsql_<version>_x64_en-US.msi` or `rsql_<version>_x64-setup.exe`
- Linux (x86_64): `.AppImage`, `.deb` or `.rpm`

The app checks for updates on startup. You can also check with the Updates button.

macOS builds are not signed yet. After copying the app to Applications, remove the quarantine flag:

```bash
xattr -dr com.apple.quarantine /Applications/rsql.app
```

## Features

### Editor

- Monaco editor with completion for schemas, tables, views, columns, functions and table aliases, plus SQL snippets and a formatter
- Tabs, a split view for a second query, and saved workspaces
- History of the last 500 queries in the current session
- Statement timeout per tab
- Scripts with several statements (the last result set is shown)
- `EXPLAIN ANALYZE` plan viewer with time, estimated and actual rows, and cost for each node

### Results

- Canvas-based grid ([Glide Data Grid](https://github.com/glideapps/glide-data-grid))
- Results over 2,000 rows stay in the backend and load page by page as you scroll, up to 1,000,000 rows or 256 MB
- On results up to 2,000 rows: search, a single-row record view, inline editing, foreign key navigation and diffing against a pinned result
- Inline editing works on single-table queries that select the primary key, and applies all changes in one transaction
- Export to CSV, JSON, SQL `INSERT`, Markdown or XML; large results export to CSV
- CSV import into a table, with column mapping
- Map view for geometry values (WKT, GeoJSON, EWKB points) on OpenStreetMap

### Schema

- Sidebar with databases, roles, tablespaces, schemas, tables, views, materialized views and functions, and for each table its columns, indexes, constraints, triggers, rules and RLS policies
- Object properties with DDL, a structure editor that generates `ALTER TABLE`, and maintenance actions such as `VACUUM`, `ANALYZE` and `REINDEX`
- ER diagram per schema, exportable as SVG
- Diff between two schemas
- Command palette (Cmd/Ctrl+K)

### Server

- Performance monitor: activity, table statistics, locks, index advisor, bloat and the slowest queries of the session
- Roles and grants, extensions, enums, server settings, LISTEN/NOTIFY
- Built-in terminal (Cmd/Ctrl+`)

### Connections

- Saved connections, connection strings and SSL
- SSH tunnels with password or key file authentication; the server's host key is verified before authenticating
- Passwords are stored unencrypted in the app's local database

## How it works

- User SQL runs through the simple query protocol of `tokio-postgres`, so values arrive as text and are passed on without conversion.
- Results cross the Tauri IPC boundary as packed strings with control-character separators, not as nested JSON arrays.
- Large results are packed into pages in Rust and share one 256 MB memory budget. The grid requests pages by index and keeps the 24 closest to the viewport.
- Each connection has separate pools for queries (8 connections) and metadata (4), so loading the schema does not wait for a long query.

## Building from source

You need Node.js 24.21.0 (`.node-version`), Yarn 1, Rust 1.95.0 (pinned in `src-tauri/rust-toolchain.toml`) and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS.

```bash
yarn install --frozen-lockfile
yarn tauri dev
```

Updater artifacts need the release signing key, so turn them off for a local build:

```bash
yarn tauri build --no-sign --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

The installers end up in `src-tauri/target/release/bundle/`.

## Tests

```bash
yarn test
cd src-tauri && cargo test
```

The PostgreSQL integration tests are ignored by default and need a database:

```bash
cd src-tauri
export RSQL_TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
cargo test --test row_mutations --test schema_index -- --ignored
cargo test --bin rsql -- --ignored
```

## Releasing

Pushing a `v*` tag runs `.github/workflows/release.yml`. It builds every platform, signs the updater artifacts and uploads everything to a draft release, which you publish once the builds finish. The app reads `https://github.com/rust-dd/rust-sql/releases/latest/download/latest.json` to find updates.

Repository secrets:

- Updater: `TAURI_UPDATER_PUBLIC_KEY`, `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (optional)
- Windows: `WINDOWS_CERTIFICATE` (base64 `.pfx`), `WINDOWS_CERTIFICATE_PASSWORD`, `WINDOWS_TIMESTAMP_URL` (optional, defaults to DigiCert)
- Linux: `TAURI_SIGNING_RPM_KEY` (ASCII-armored GPG key), `TAURI_SIGNING_RPM_KEY_PASSPHRASE` (optional), `APPIMAGETOOL_SIGN_PASSPHRASE`, `SIGN_KEY` (optional, GPG key ID for the AppImage)

## License

[MIT](LICENSE)
