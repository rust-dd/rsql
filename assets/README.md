# Application icon

`app-icon.png` is the transparent master for the desktop application icon: a rust-orange gear around an ivory R and database stack on a charcoal tile.

From the repository root, run:

```bash
yarn install --frozen-lockfile
yarn icons:generate
```

This uses the project's Tauri CLI to replace the desktop PNG, Windows ICO and macOS ICNS files in `src-tauri/icons`. Mobile assets are generated in a temporary directory and discarded. Keep the master and generated desktop files together when changing the icon.

The existing `bundle.icon` configuration consumes these files on the next application build. An already installed app keeps its old icon until it is replaced with a new build.
