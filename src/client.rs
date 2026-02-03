use anyhow::Result;
use std::path::Path;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

// Assuming you have a utils module available, otherwise use std::env
use crate::utils;

pub async fn send<P: AsRef<Path>>(socket_path: Option<P>, command: &str) -> Result<()> {
    // FIX: Use map_or_else to satisfy Clippy
    let path = socket_path.map_or_else(
        // 1. Default case (None) - runs if socket_path is None
        || {
            // Try to get path from utils, or fallback to a sensible default
            // If utils::get_socket_path returns a String, we convert to PathBuf
            utils::get_socket_path(None).into()
        },
        // 2. Map case (Some) - runs if socket_path is Some
        |p| p.as_ref().to_path_buf(),
    );

    let mut stream = UnixStream::connect(path).await?;
    stream.write_all(command.as_bytes()).await?;
    stream.shutdown().await?;

    Ok(())
}
