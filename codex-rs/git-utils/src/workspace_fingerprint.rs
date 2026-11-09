//! Bounded fingerprint of Git-visible file contents, never a claim about ignored
//! build inputs, environment variables, services, or the entire operating system.

use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

const MAX_FILES: usize = 10_000;
const MAX_BYTES: usize = 100 * 1024 * 1024;

pub async fn workspace_fingerprint(cwd: &Path) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(3), fingerprint(cwd))
        .await
        .ok()
        .flatten()
}

async fn git(cwd: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        .args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            crate::SAFE_BARE_REPOSITORY_CONFIG,
        ])
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut output = Vec::new();
    child
        .stdout
        .take()?
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut output)
        .await
        .ok()?;
    if output.len() > 2 * 1024 * 1024 {
        return None;
    }
    if !child.wait().await.ok()?.success() {
        return None;
    }
    Some(output)
}

async fn fingerprint(cwd: &Path) -> Option<String> {
    // Restrict capture to a repository root. Subdirectory and remote executions
    // explicitly have unknown freshness rather than silently hashing a partial tree.
    if git(cwd, &["rev-parse", "--show-prefix"]).await? != b"\n" {
        return None;
    }
    let listing = git(
        cwd,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )
    .await?;
    let mut names: Vec<_> = listing
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.len() > MAX_FILES {
        return None;
    }
    let mut hash = blake3::Hasher::new();
    hash.update(b"codex-git-visible-workspace-v1\0");
    let mut total = 0usize;
    for name in names {
        let relative = Path::new(std::str::from_utf8(name).ok()?);
        if relative
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return None;
        }
        let path = cwd.join(relative);
        hash.update(&(name.len() as u64).to_le_bytes());
        hash.update(name);
        // Reject symlinks in every component, including a directory replaced by a link.
        let mut parent = cwd.to_path_buf();
        for component in relative.components() {
            parent.push(component);
            match tokio::fs::symlink_metadata(&parent).await {
                Ok(meta) if meta.file_type().is_symlink() => return None,
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => return None,
            }
        }
        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                hash.update(b"missing");
                continue;
            }
            Err(_) => return None,
        };
        if !meta.is_file() {
            return None;
        }
        total = total.checked_add(usize::try_from(meta.len()).ok()?)?;
        if total > MAX_BYTES {
            return None;
        }
        let mut bytes = Vec::new();
        tokio::fs::File::open(&path)
            .await
            .ok()?
            .take(meta.len() + 1)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() as u64 != meta.len() {
            return None;
        }
        hash.update(b"file");
        hash.update(&meta.len().to_le_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            hash.update(&(meta.permissions().mode() & 0o111).to_le_bytes());
        }
        hash.update(&bytes);
    }
    // A changing file set is not a stable snapshot.
    if git(
        cwd,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )
    .await?
        != listing
    {
        return None;
    }
    Some(hash.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fingerprint_tracks_dirty_untracked_and_deleted_contents() {
        let dir = tempfile::tempdir().unwrap();
        assert!(git(dir.path(), &["init", "-q"]).await.is_some());
        tokio::fs::write(dir.path().join("file"), "one")
            .await
            .unwrap();
        git(dir.path(), &["add", "file"]).await.unwrap();
        let first = workspace_fingerprint(dir.path()).await.unwrap();
        tokio::fs::write(dir.path().join("file"), "two")
            .await
            .unwrap();
        let dirty = workspace_fingerprint(dir.path()).await.unwrap();
        assert_ne!(first, dirty);
        tokio::fs::write(dir.path().join("new"), "untracked")
            .await
            .unwrap();
        let untracked = workspace_fingerprint(dir.path()).await.unwrap();
        assert_ne!(dirty, untracked);
        tokio::fs::remove_file(dir.path().join("file"))
            .await
            .unwrap();
        assert_ne!(untracked, workspace_fingerprint(dir.path()).await.unwrap());
    }
}
