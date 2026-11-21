mod relaunch;
pub use relaunch::Relaunch;

use gpui_kit::{App, ClipboardItem, Window};
use std::{
    path::{Path, PathBuf},
    pin::Pin,
};

pub struct DesktopServices;

impl DesktopServices {
    pub fn pick_migration_path(
        window: &Window,
        manifest: bool,
    ) -> Pin<Box<dyn Future<Output = Option<PathBuf>>>> {
        let dialog = rfd::AsyncFileDialog::new().set_parent(window);
        Box::pin(async move {
            let selection = if manifest {
                dialog
                    .add_filter("Migration manifest", &["json"])
                    .pick_file()
                    .await
            } else {
                dialog.pick_folder().await
            };
            selection.map(|file| file.path().to_owned())
        })
    }

    pub fn pick_csv(
        window: &Window,
        extensions: &[&str],
    ) -> Pin<Box<dyn Future<Output = Option<PathBuf>>>> {
        let response = rfd::AsyncFileDialog::new()
            .set_parent(window)
            .add_filter("CSV files", extensions)
            .pick_file();
        Box::pin(async move { response.await.map(|f| f.path().to_owned()) })
    }

    pub fn save_path(
        cx: &App,
        filename: &str,
    ) -> impl Future<Output = Result<Option<PathBuf>, String>> + Send + use<> {
        let prompt = cx.prompt_for_new_path(Path::new("."), Some(filename));
        async move {
            prompt
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())
        }
    }

    pub fn write_clipboard(cx: &mut App, item: ClipboardItem) {
        cx.write_to_clipboard(item);
    }

    pub fn read_clipboard(cx: &App) -> Option<ClipboardItem> {
        cx.read_from_clipboard()
    }

    pub fn open_url(cx: &App, url: &str) {
        cx.open_url(url);
    }
}
