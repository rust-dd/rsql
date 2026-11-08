# RSQL

**A desktop PostgreSQL client for macOS, Windows and Linux.**

Write SQL, edit data, explore schemas and inspect query plans in one app. RSQL is built with Rust, Tauri and React, and is free and open source under the MIT license. No account required. No telemetry.

[Download](https://github.com/rust-dd/rsql/releases/latest) · [First query](#run-your-first-query) · [Features](#features) · [Development](#development) · [Report an issue](https://github.com/rust-dd/rsql/issues)

![RSQL desktop app with the schema explorer, SQL editor and results grid](docs/rsql.png)

## Install

Choose an installer from the [latest release](https://github.com/rust-dd/rsql/releases/latest):

| Platform | Architecture | Download |
| --- | --- | --- |
| macOS | Apple Silicon | `rsql_<version>_aarch64.dmg` |
| macOS | Intel | `rsql_<version>_x64.dmg` |
| Windows | x64 | `.msi` or `-setup.exe` |
| Linux | x86_64 | `.AppImage`, `.deb` or `.rpm` |

On macOS, open the DMG and drag RSQL into Applications. Signing and notarization details are documented in the [macOS release guide](docs/macos-release.md).

RSQL checks for updates on startup. Use **Updates** in the top bar to check manually.

## Run your first query

1. Open the command palette with **Cmd/Ctrl+K** and choose **New Connection**.
2. Enter your host, port, database, username and password, or paste a connection URL such as `postgresql://user:password@localhost:5432/mydb`. Enable SSL or configure an SSH tunnel if your server requires it.
3. Click **Connect**, then select the connection for your query tab if needed.
4. Enter a query and press **Cmd/Ctrl+Enter**, or click **Execute**:

   ```sql
   SELECT current_database(), current_user, version();
   ```

Expand the sidebar to browse schemas and tables. Use the command palette to find database objects, open another query tab, or save and load a workspace.

## Features

### Write and organize SQL

- **Schema-aware completion:** Monaco editor with suggestions for schemas, tables, views, columns, functions and table aliases, plus SQL snippets and formatting.
- **Tabs and split view:** work on several queries, compare queries side by side, and save workspaces for later.
- **Saved queries and history:** keep queries for reuse and browse the last 500 executions in the current session.
- **Execution controls:** cancel a running query or set a statement timeout per tab. Scripts can contain multiple statements; the last result set is displayed.
- **Query plans:** inspect `EXPLAIN ANALYZE` output with timing, cost, and estimated versus actual rows for each node. Explain executes the query to collect measurements.

### Inspect and edit data

- **Results grid:** canvas rendering with pages loaded from Rust as you scroll through large results.
- **Data editing:** update and delete rows in eligible single-table results, then apply changes together in one transaction. The query must include the table's primary key.
- **Result tools:** search cells, inspect a row in Record view, follow foreign keys, and compare results against a pinned snapshot.
- **Import and export:** import CSV with column mapping; export to CSV, JSON, SQL `INSERT`, Markdown or XML. Paged results support CSV export.
- **Geometry preview:** view WKT, GeoJSON and EWKB point values on an OpenStreetMap map.

In edit mode, type `NULL` in any letter case for SQL NULL, or `"NULL"` for literal text. Empty input remains an empty string; cell-to-cell pastes preserve literal text. See [result limits](#result-limits) for tools available on larger queries.

### Explore schemas and manage PostgreSQL

- **Object browser:** databases, roles, tablespaces, schemas, tables, views, materialized views and functions, with columns, indexes, constraints, triggers, rules and row-level security policies.
- **Schema tools:** inspect DDL, generate `ALTER TABLE` statements in the structure editor, compare schemas, and export schema ER diagrams as SVG.
- **Server tools:** roles and grants, extensions, enums, server settings, `LISTEN`/`NOTIFY`, and a built-in terminal.
- **Performance monitor:** activity, locks, table statistics, index advice, bloat estimates and the slowest queries of the current session.
- **Maintenance:** run actions such as `VACUUM`, `ANALYZE` and `REINDEX` from object properties.

### Manage connections

- Save and rename connections, paste PostgreSQL connection URLs, and enable SSL.
- Connect through SSH with a password or key file. RSQL verifies the server's host key before authenticating.
- Give connections on the same server a shared label while keeping the server address visible.
- Switch between light and dark themes; the choice is restored on restart.

## Keyboard shortcuts

Use **Cmd** on macOS and **Ctrl** on Windows and Linux.

| Action | Shortcut |
| --- | --- |
| Execute SQL in the focused editor | Cmd/Ctrl+Enter |
| Explain the query | Cmd/Ctrl+Shift+Enter |
| Cancel the running query | Cmd/Ctrl+. |
| Open the command palette | Cmd/Ctrl+K |
| Open a terminal tab | Cmd/Ctrl+` |
| Close the active tab | Cmd/Ctrl+W |

## Result limits

The main query pane uses 2,000-row pages by default. Larger results stay in Rust and are fetched by the grid as needed.

| Capability | Up to 2,000 rows | More than 2,000 rows |
| --- | --- | --- |
| Browse results | Full result in the grid | Pages loaded as you scroll |
| Search, Record view, editing, foreign key navigation and result diff | Available where the query supports them | Narrow the query to use these tools |
| Export | CSV, JSON, SQL `INSERT`, Markdown, XML | CSV of the retained result |
| Map view | Detected geometry values | First page only |

The backend retains at most **1,000,000 rows per result**, within a **256 MiB budget shared by cached results and running paged queries**. Results that reach a limit are marked **truncated**. These limits bound retained data; they do not add a SQL `LIMIT` or stop the remaining statements in a script. Narrow the query when you need rows beyond the retained result.

## Local data and credentials

Saved connections, queries and workspaces live in a local `rsql.db` database. Release builds use the operating system's application data directory; debug builds use the working directory. Open query tabs and editor contents are restored from local WebView storage. Query history is held in memory for the current session.

**Database and SSH passwords are currently stored unencrypted in `rsql.db`.** Treat that file and its backups as sensitive. When older connection records are migrated to stable IDs, RSQL first creates a sibling backup named `rsql.db.<timestamp>.bak`.

## Development

### Prerequisites

- Node.js **24.21.0**, pinned in [`.node-version`](.node-version).
- Yarn **1.x**.
- Rust **1.95.0**, pinned in [`src-tauri/rust-toolchain.toml`](src-tauri/rust-toolchain.toml).
- The [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/) for your operating system.
- Access to a PostgreSQL server to use the app. CI runs integration tests against PostgreSQL **16, 17 and 18**.

### Run locally

```bash
git clone https://github.com/rust-dd/rsql.git
cd rsql
yarn install --frozen-lockfile
yarn tauri dev
```

`yarn tauri dev` starts both Vite and the desktop app. `yarn dev` starts only the frontend server; database access requires the Tauri backend.

### Build an installer

For a local build without release signing keys, disable updater artifacts:

```bash
yarn tauri build --no-sign --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

Installers are written to `src-tauri/target/release/bundle/`. Automatic updates are disabled in builds without `TAURI_UPDATER_PUBLIC_KEY`.

### Checks and tests

Run the frontend checks from the repository root:

```bash
node scripts/check-version.mjs
yarn check:ci
yarn typecheck
yarn test
yarn build
```

Run Rust checks from `src-tauri` so the pinned toolchain is used:

```bash
cd src-tauri
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Database integration tests are ignored by default. To run them, point `RSQL_TEST_DATABASE_URL` at a disposable PostgreSQL database. From the repository root, in a Bash-compatible shell:

```bash
cd src-tauri
export RSQL_TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
cargo test --locked --test row_mutations --test schema_index --test foreign_keys -- --ignored
cargo test --locked --bin rsql -- --ignored
```

### Project layout

| Path | Contents |
| --- | --- |
| [`src/`](src/) | React UI, Zustand stores, Monaco completion and Tauri client calls |
| [`src-tauri/`](src-tauri/) | Rust backend, PostgreSQL driver, SSH tunnels and local storage |
| [`tests/`](tests/) | Shared fixtures and browser performance harnesses |
| [`scripts/`](scripts/) | Version checks, dependency audits and icon generation |
| [`docs/`](docs/) | Project documentation and release setup |
| [`website/`](website/) | Separate website with an interactive SQL demo |

The desktop icon source is [`assets/app-icon.png`](assets/app-icon.png). Run `yarn icons:generate` to regenerate platform icons; see the [icon notes](assets/README.md).

### Architecture

The React UI talks to Rust through Tauri commands. The PostgreSQL driver uses `tokio-postgres`'s simple query protocol, preserving database values as text. Results cross the IPC boundary as packed strings and are decoded into grid pages in the frontend.

Each saved connection has separate pools for user queries (up to 8 connections) and metadata (up to 4), so schema browsing does not queue behind a long query. Local persistence uses libSQL.

## Contributing

[Open an issue](https://github.com/rust-dd/rsql/issues) for bugs or feature requests. For bugs, include your RSQL version, operating system, PostgreSQL version and steps to reproduce, with credentials and sensitive data removed.

For code changes, run the relevant checks above and describe the problem and resulting behavior in your pull request. Use a purpose-based branch name such as `fix/query-cancellation` or `feat/result-export`, and Conventional Commit subjects such as `fix: handle cancelled queries`.

## Releases

Pushing a `v*` tag runs the [release workflow](.github/workflows/release.yml). It verifies version consistency, runs frontend and Rust checks, builds the supported platforms, and uploads signed updater artifacts to a draft GitHub release. Publish the draft after the builds and signing checks finish. The updater endpoint is configured in [`tauri.conf.json`](src-tauri/tauri.conf.json).

<details>
<summary>Release signing configuration</summary>

Configure these GitHub repository secrets before releasing:

- **Updater:** `TAURI_UPDATER_PUBLIC_KEY`, `TAURI_SIGNING_PRIVATE_KEY`, and optionally `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- **macOS:** `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (app-specific password) and `APPLE_TEAM_ID`. Set the repository variable `MACOS_SIGNING_ENABLED` to `true` after following the [macOS signing guide](docs/macos-release.md).
- **Windows:** `WINDOWS_CERTIFICATE` (base64 `.pfx`), `WINDOWS_CERTIFICATE_PASSWORD`, and optionally `WINDOWS_TIMESTAMP_URL` (defaults to DigiCert).
- **Linux:** `TAURI_SIGNING_RPM_KEY` (ASCII-armored GPG key), `APPIMAGETOOL_SIGN_PASSPHRASE`, and optionally `TAURI_SIGNING_RPM_KEY_PASSPHRASE` and `SIGN_KEY` (AppImage GPG key ID).

</details>

## License

[MIT](LICENSE) — free to use, modify and distribute.
