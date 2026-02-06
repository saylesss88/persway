use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::utils;
use std::path::Path;

pub async fn send<P: AsRef<Path>>(socket_path: Option<P>, command: &str) -> Result<()> {
    let path = socket_path.map_or_else(
        || utils::get_socket_path(None).into(),
        |p| p.as_ref().to_path_buf(),
    );

    let mut stream = UnixStream::connect(path).await?;
    stream.write_all(command.as_bytes()).await?;
    stream.write_all(b"\n").await?; // ensure newline, in case daemon cares

    // Read the reply line
    let (read_half, _) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut resp = String::new();

    reader.read_line(&mut resp).await?;

    let resp = resp.trim_end();

    match resp {
        "success" => Ok(()),
        s if s.starts_with("fail:") => {
            let msg = s.strip_prefix("fail:").unwrap().trim();
            anyhow::bail!("{msg}");
        }
        _ => anyhow::bail!("unexpected response: {resp}"),
    }
}
