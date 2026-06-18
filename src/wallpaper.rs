//! Wallpaper management: only compiled with the `wallpaper` feature.

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
    handles: Vec<JoinHandle<Result<(), RenderError>>>,
}

impl WallpaperHandle {
    /// Signals the running renderer to stop and awaits its thread.
    pub async fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        for handle in self.handles {
            let _ = handle.await;
        }
    }
}

/// Spawn wallpaper renderer for a single output.
pub fn spawn_for_output(path: PathBuf, output: String) -> WallpaperHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let handle = tokio::task::spawn_blocking(move || {
        render_wallpaper(&path, Some(output.as_str()), &stop_clone)
    });

    WallpaperHandle {
        stop,
        handles: vec![handle],
    }
}

/// Spawn a new wallpaper renderer, returning a handle to cancel it later.
///
/// If `output` is `None` the compositor chooses the output.
pub async fn spawn(path: PathBuf, output: Option<String>) -> WallpaperHandle {
    let stop = Arc::new(AtomicBool::new(false));

    let outputs: Vec<Option<String>> = match &output {
        // Specific output requested — just use that one
        Some(name) => vec![Some(name.clone())],
        // No output specified, enumerate all active outputs via sway IPC
        None => {
            match async {
                let mut conn = swayipc_async::Connection::new().await?;
                conn.get_outputs().await
            }
            .await
            {
                Ok(outs) => outs
                    .into_iter()
                    .filter(|o| o.active)
                    .map(|o| Some(o.name))
                    .collect(),
                Err(e) => {
                    log::warn!(
                        "failed to enumerate outputs via sway IPC: {e}; falling back to compositor default"
                    );
                    vec![None]
                }
            }
        }
    };

    let handles = outputs
        .into_iter()
        .map(|out_name| {
            let path = path.clone();
            let stop_clone = Arc::clone(&stop);
            tokio::task::spawn_blocking(move || {
                render_wallpaper(&path, out_name.as_deref(), &stop_clone)
            })
        })
        .collect();

    WallpaperHandle { stop, handles }
}
