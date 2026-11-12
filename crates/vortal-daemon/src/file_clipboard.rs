use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};
use vortal_proto::{ClipboardData, ClipboardOffer, Message};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const CHUNK_BYTES: usize = 16 * 1024;
const MIME: &str = "application/vnd.vortal.files";

#[derive(Serialize, Deserialize)]
struct FileEntry {
    name: String,
    size: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
}

#[derive(Default)]
pub struct IncomingFiles {
    offer: Option<ClipboardOffer>,
    next_chunk: u32,
    bytes: Vec<u8>,
}

impl IncomingFiles {
    pub fn accept(&mut self, message: Message) -> Option<(u64, Vec<u8>)> {
        match message {
            Message::ClipboardOffer(offer) => {
                *self = Self::default();
                if offer.mime_type == MIME
                    && offer.total_size <= MAX_BYTES as u64
                    && offer.chunk_count > 0
                    && offer.chunk_count <= 4096
                {
                    self.bytes.reserve(offer.total_size as usize);
                    self.offer = Some(offer);
                }
                None
            }
            Message::ClipboardData(chunk) => {
                let offer = self.offer.as_ref()?;
                if chunk.transfer_id != offer.transfer_id
                    || chunk.chunk_index != self.next_chunk
                    || chunk.data.len() > CHUNK_BYTES
                    || self.bytes.len() + chunk.data.len() > offer.total_size as usize
                {
                    *self = Self::default();
                    return None;
                }
                self.bytes.extend_from_slice(&chunk.data);
                self.next_chunk += 1;
                if chunk.final_chunk
                    && self.next_chunk == offer.chunk_count
                    && self.bytes.len() == offer.total_size as usize
                {
                    let id = offer.transfer_id;
                    let bytes = std::mem::take(&mut self.bytes);
                    *self = Self::default();
                    Some((id, bytes))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name != "."
        && name != ".."
        && !name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch.is_control())
}

pub async fn read_selection() -> Result<Option<(String, Vec<PathBuf>)>> {
    let types = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::process::Command::new("wl-paste")
            .arg("--list-types")
            .output(),
    )
    .await??;
    if !types.status.success()
        || !String::from_utf8_lossy(&types.stdout)
            .lines()
            .any(|line| line == "text/uri-list")
    {
        return Ok(None);
    }
    let output = tokio::time::timeout(
        Duration::from_secs(1),
        tokio::process::Command::new("wl-paste")
            .args(["--no-newline", "--type", "text/uri-list"])
            .output(),
    )
    .await??;
    if !output.status.success() || output.stdout.len() > 16 * 1024 {
        return Ok(None);
    }
    let signature = String::from_utf8(output.stdout)?;
    let mut paths = Vec::new();
    for line in signature
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let path = url::Url::parse(line)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("invalid file URI"))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !valid_name(name) || !path.is_file() {
            return Ok(None);
        }
        paths.push(path);
        if paths.len() > 16 {
            return Ok(None);
        }
    }
    Ok((!paths.is_empty()).then_some((signature, paths)))
}

pub async fn send_files(sender: &broadcast::Sender<Message>, paths: &[PathBuf]) -> Result<()> {
    let mut entries = Vec::with_capacity(paths.len());
    let mut contents = Vec::with_capacity(paths.len());
    let mut total = 0usize;
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !valid_name(name) {
            bail!("invalid clipboard filename");
        }
        let metadata = tokio::fs::metadata(path).await?;
        let size = usize::try_from(metadata.len()).context("file too large")?;
        total = total.checked_add(size).context("file set too large")?;
        if total > MAX_BYTES - 16 * 1024 {
            bail!("clipboard files exceed 32 MiB");
        }
        entries.push(FileEntry {
            name: name.into(),
            size,
            kind: None,
        });
        contents.push(tokio::fs::read(path).await?);
    }
    let header = serde_json::to_vec(&entries)?;
    if header.len() > 16 * 1024 {
        bail!("file metadata too large");
    }
    let mut archive = Vec::with_capacity(4 + header.len() + total);
    archive.extend_from_slice(&(header.len() as u32).to_le_bytes());
    archive.extend_from_slice(&header);
    for content in contents {
        archive.extend_from_slice(&content);
    }
    if archive.len() > MAX_BYTES {
        bail!("clipboard files exceed 32 MiB");
    }
    let transfer_id = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros() as u64;
    let count = archive.len().div_ceil(CHUNK_BYTES) as u32;
    sender.send(Message::ClipboardOffer(ClipboardOffer {
        transfer_id,
        mime_type: MIME.into(),
        total_size: archive.len() as u64,
        chunk_count: count,
    }))?;
    for (index, data) in archive.chunks(CHUNK_BYTES).enumerate() {
        sender.send(Message::ClipboardData(ClipboardData {
            transfer_id,
            chunk_index: index as u32,
            data: data.to_vec(),
            final_chunk: index as u32 + 1 == count,
        }))?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

pub async fn save_files(
    transfer_id: u64,
    archive: Vec<u8>,
    last_files: Arc<Mutex<Option<String>>>,
) -> Result<()> {
    if archive.len() < 4 {
        bail!("missing file metadata");
    }
    let header_size = u32::from_le_bytes(archive[..4].try_into()?) as usize;
    if header_size > 16 * 1024 || archive.len() < 4 + header_size {
        bail!("invalid file metadata");
    }
    let entries: Vec<FileEntry> = serde_json::from_slice(&archive[4..4 + header_size])?;
    if entries.is_empty()
        || entries.len() > 16
        || entries.iter().any(|entry| {
            !valid_name(&entry.name) || !matches!(entry.kind.as_deref(), None | Some("image"))
        })
        || (entries
            .iter()
            .any(|entry| entry.kind.as_deref() == Some("image"))
            && entries.len() != 1)
    {
        bail!("invalid file list");
    }
    let total: usize = entries.iter().try_fold(0usize, |sum, entry| {
        sum.checked_add(entry.size).context("file set too large")
    })?;
    if total != archive.len() - 4 - header_size {
        bail!("file data length mismatch");
    }
    let image = entries[0].kind.as_deref() == Some("image");
    if image && !archive[4 + header_size..].starts_with(b"\x89PNG\r\n\x1a\n") {
        bail!("invalid PNG clipboard image");
    }
    let data_dir = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .context("data directory unavailable")?;
    let root = data_dir
        .join("vortal/clipboard")
        .join(transfer_id.to_string());
    let mut offset = 4 + header_size;
    let mut uris = String::new();
    for (index, entry) in entries.iter().enumerate() {
        let directory = root.join(index.to_string());
        tokio::fs::create_dir_all(&directory).await?;
        let path = directory.join(&entry.name);
        tokio::fs::write(&path, &archive[offset..offset + entry.size]).await?;
        offset += entry.size;
        uris.push_str(
            url::Url::from_file_path(&path)
                .map_err(|_| anyhow::anyhow!("invalid output path"))?
                .as_str(),
        );
        uris.push_str("\r\n");
    }
    use tokio::io::AsyncWriteExt;
    let mime = if image { "image/png" } else { "text/uri-list" };
    let clipboard_bytes = if image {
        &archive[4 + header_size..]
    } else {
        uris.as_bytes()
    };
    let mut child = tokio::process::Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(clipboard_bytes).await?;
    }
    let status = tokio::time::timeout(Duration::from_secs(2), child.wait()).await??;
    if !status.success() {
        bail!("could not set Wayland file clipboard");
    }
    *last_files.lock().await = if image { None } else { Some(uris) };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{send_files, valid_name, IncomingFiles};
    #[test]
    fn rejects_paths_and_control_characters() {
        assert!(valid_name("documento.txt"));
        for name in ["../x", "a\\b", "..", "", "a\n"] {
            assert!(!valid_name(name));
        }
    }

    #[tokio::test]
    async fn transfers_multiple_files_in_order() -> anyhow::Result<()> {
        let directory = std::env::temp_dir().join(format!(
            "vortal-files-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        tokio::fs::create_dir_all(&directory).await?;
        let first = directory.join("um.txt");
        let second = directory.join("dois.txt");
        tokio::fs::write(&first, b"abc").await?;
        tokio::fs::write(&second, b"defg").await?;
        let (sender, mut receiver) = tokio::sync::broadcast::channel(8);
        let task = tokio::spawn(async move { send_files(&sender, &[first, second]).await });
        let mut incoming = IncomingFiles::default();
        let archive = loop {
            let message =
                tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv()).await??;
            if let Some((_id, archive)) = incoming.accept(message) {
                break archive;
            }
        };
        task.await??;
        let header_size = u32::from_le_bytes(archive[..4].try_into()?) as usize;
        let names: serde_json::Value = serde_json::from_slice(&archive[4..4 + header_size])?;
        assert_eq!(names[0]["name"], "um.txt");
        assert_eq!(names[1]["name"], "dois.txt");
        assert_eq!(&archive[4 + header_size..], b"abcdefg");
        tokio::fs::remove_dir_all(directory).await?;
        Ok(())
    }
}
