//! Wallpaper management — only compiled with the `wallpaper` feature.

use randpaper_lib::{errors::RenderError, layer::render_wallpaper};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::task::JoinHandle;

/// Owns the currently-running wallpaper task and its stop flag.
pub struct WallpaperHandle {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Result<(), RenderError>>,
}

impl WallpaperHandle {
    /// Signals the running renderer to stop and awaits its thread.
    pub async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        // Ignore the result — we just want it gone.
        let _ = self.handle.await;
    }
}

/// Spawn a new wallpaper renderer, returning a handle to cancel it later.
///
/// If `output` is `None` the compositor chooses the output.
pub fn spawn(path: PathBuf, output: Option<String>) -> WallpaperHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let handle = tokio::task::spawn_blocking(move || {
        render_wallpaper(&path, output.as_deref(), &stop_clone)
    });

    WallpaperHandle { stop, handle }
}
