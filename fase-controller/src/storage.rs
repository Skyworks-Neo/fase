use fase_api::sha256_hex;
use fase_api::{ArtifactKind, ArtifactSpec};
use object_store::{
    ObjectStore, ObjectStoreExt, PutMode, PutPayload,
    aws::{AmazonS3Builder, S3ConditionalPut},
    path::Path,
};
use std::{
    env,
    path::{Path as FsPath, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct Store {
    inner: Arc<dyn ObjectStore>,
    prefix: String,
}

impl Store {
    pub fn from_env() -> Result<Self, String> {
        let bucket = env::var("FASE_S3_BUCKET").map_err(|_| "FASE_S3_BUCKET is required")?;
        let endpoint = env::var("FASE_S3_ENDPOINT").map_err(|_| "FASE_S3_ENDPOINT is required")?;
        let allow_insecure = env::var("FASE_ALLOW_INSECURE_S3")
            .map(|value| value == "true")
            .unwrap_or(false);
        if !endpoint.starts_with("https://") && !endpoint.starts_with("http://") {
            return Err("FASE_S3_ENDPOINT must use http or https".into());
        }
        if endpoint.starts_with("http://") && !allow_insecure {
            return Err(
                "FASE_S3_ENDPOINT must use https unless FASE_ALLOW_INSECURE_S3=true".into(),
            );
        }
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(bucket)
            .with_endpoint(&endpoint)
            .with_allow_http(endpoint.starts_with("http://"))
            .with_virtual_hosted_style_request(false)
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            .with_region(env::var("FASE_S3_REGION").unwrap_or_else(|_| "us-east-1".into()));
        let access = env::var("FASE_S3_ACCESS_KEY_ID");
        let secret = env::var("FASE_S3_SECRET_ACCESS_KEY");
        match (access, secret) {
            (Ok(access), Ok(secret)) => {
                builder = builder
                    .with_access_key_id(access)
                    .with_secret_access_key(secret);
                if let Ok(token) = env::var("FASE_S3_SESSION_TOKEN") {
                    builder = builder.with_token(token);
                }
            }
            (Err(_), Err(_)) => {
                if env::var("FASE_S3_ANONYMOUS")
                    .map(|value| value == "true")
                    .unwrap_or(false)
                {
                    builder = builder.with_skip_signature(true);
                } else {
                    return Err("S3 credentials are required unless FASE_S3_ANONYMOUS=true".into());
                }
            }
            _ => {
                return Err(
                    "FASE_S3_ACCESS_KEY_ID and FASE_S3_SECRET_ACCESS_KEY must be set together"
                        .into(),
                );
            }
        }
        let inner = builder.build().map_err(|error| error.to_string())?;
        Ok(Self {
            inner: Arc::new(inner),
            prefix: env::var("FASE_S3_PREFIX")
                .unwrap_or_default()
                .trim_matches('/')
                .to_string(),
        })
    }

    fn object_path(&self, name: &str) -> Result<Path, String> {
        if !valid_artifact_name(name) {
            return Err(format!("invalid Artifact name {name}"));
        }
        Path::parse(if self.prefix.is_empty() {
            format!("objects/{name}")
        } else {
            format!("{}/objects/{name}", self.prefix)
        })
        .map_err(|error| error.to_string())
    }

    pub async fn get(
        &self,
        artifact_name: &str,
        spec: &ArtifactSpec,
        kind: ArtifactKind,
        max_bytes: u64,
    ) -> Result<Vec<u8>, String> {
        if spec.kind != kind || spec.storage_ref.key != format!("objects/{artifact_name}") {
            return Err(format!(
                "Artifact {artifact_name} has invalid storage reference"
            ));
        }
        let path = self.object_path(artifact_name)?;
        let result = self
            .inner
            .get(&path)
            .await
            .map_err(|error| error.to_string())?;
        if result.range.end.saturating_sub(result.range.start) > max_bytes {
            return Err(format!(
                "artifact {artifact_name} exceeds the configured size limit"
            ));
        }
        let bytes = result.bytes().await.map_err(|error| error.to_string())?;
        let actual_name = artifact_id(&sha256_hex(&bytes), kind);
        if artifact_name != actual_name {
            return Err(format!(
                "Artifact {artifact_name} failed content identity verification"
            ));
        }
        if spec.content_digest != format!("sha256:{}", sha256_hex(&bytes))
            || spec.size_bytes != bytes.len() as i64
        {
            return Err(format!(
                "Artifact {artifact_name} failed digest or size verification"
            ));
        }
        Ok(bytes.to_vec())
    }

    pub async fn put(&self, name: &str, bytes: Vec<u8>, max_bytes: u64) -> Result<(), String> {
        if bytes.len() as u64 > max_bytes {
            return Err(format!("object {name} exceeds the configured size limit"));
        }
        let path = self.object_path(name)?;
        match self
            .inner
            .put_opts(
                &path,
                PutPayload::from(bytes.clone()),
                PutMode::Create.into(),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(object_store::Error::AlreadyExists { .. }) => {
                let existing_result = self
                    .inner
                    .get(&path)
                    .await
                    .map_err(|error| error.to_string())?;
                if existing_result
                    .range
                    .end
                    .saturating_sub(existing_result.range.start)
                    > max_bytes
                {
                    return Err(format!(
                        "immutable object {name} exceeds the configured size limit"
                    ));
                }
                let existing = existing_result
                    .bytes()
                    .await
                    .map_err(|error| error.to_string())?;
                if existing.as_ref() == bytes {
                    Ok(())
                } else {
                    Err(format!(
                        "immutable object {name} already has different content"
                    ))
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Return the immutable object identity for an artifact.
///
/// The logical label is deliberately excluded. Two callers may publish the
/// same bytes under different Claim labels while sharing this Artifact.
pub fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn valid_artifact_name(name: &str) -> bool {
    name.strip_prefix("art-").is_some_and(valid_sha256)
}

pub fn artifact_id(sha256: &str, kind: ArtifactKind) -> String {
    let label = match kind {
        ArtifactKind::File => "file",
        ArtifactKind::Tree => "tree",
    };
    let bytes = format!("fase-artifact-v1\0{sha256}\0{label}");
    format!("art-{}", sha256_hex(bytes.as_bytes()))
}

pub fn collect(path: &FsPath, kind: ArtifactKind) -> Result<Vec<u8>, String> {
    match kind {
        ArtifactKind::File => {
            let meta = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(format!("{} is not a regular file", path.display()));
            }
            std::fs::read(path).map_err(|error| error.to_string())
        }
        ArtifactKind::Tree => pack_directory(path),
    }
}

pub fn checked_output_path(root: &FsPath, relative: &str) -> Result<PathBuf, String> {
    if !fase_api::valid_path(relative) {
        return Err(format!("unsafe output path {relative}"));
    }
    let mut path = root.to_path_buf();
    for part in FsPath::new(relative).components() {
        path.push(part.as_os_str());
        let meta = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "output path traverses a symlink: {}",
                path.display()
            ));
        }
    }
    Ok(path)
}

fn pack_directory(root: &FsPath) -> Result<Vec<u8>, String> {
    if !std::fs::symlink_metadata(root)
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return Err(format!("{} is not a directory", root.display()));
    }
    let mut paths = Vec::new();
    visit(root, root, &mut paths)?;
    paths.sort();
    let mut archive = tar::Builder::new(Vec::new());
    for rel in paths {
        let path = root.join(&rel);
        let meta = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        let mut header = tar::Header::new_gnu();
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        let mode = if executable(&meta) { 0o755 } else { 0o644 };
        if meta.is_dir() {
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            header.set_cksum();
            archive
                .append_data(&mut header, &rel, std::io::empty())
                .map_err(|error| error.to_string())?;
        } else if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&path).map_err(|e| e.to_string())?;
            if !safe_link_target(&rel, &target) {
                return Err(format!("unsafe symlink {}", path.display()));
            }
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_mode(0o777);
            header.set_link_name(&target).map_err(|e| e.to_string())?;
            header.set_cksum();
            archive
                .append_data(&mut header, &rel, std::io::empty())
                .map_err(|e| e.to_string())?;
        } else {
            header.set_entry_type(tar::EntryType::Regular);
            header.set_size(meta.len());
            header.set_mode(mode);
            header.set_cksum();
            archive
                .append_data(
                    &mut header,
                    &rel,
                    std::fs::File::open(&path).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
        }
    }
    archive.into_inner().map_err(|error| error.to_string())
}

fn visit(root: &FsPath, dir: &FsPath, found: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if !(meta.is_dir() || meta.is_file() || meta.file_type().is_symlink()) {
            return Err(format!("unsafe output {}", path.display()));
        }
        found.push(
            path.strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_path_buf(),
        );
        if meta.is_dir() {
            visit(root, &path, found)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}
#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    false
}

pub fn materialize(
    root: &FsPath,
    path: &str,
    kind: ArtifactKind,
    bytes: &[u8],
) -> Result<(), String> {
    if !fase_api::valid_path(path) {
        return Err(format!("unsafe input path {path}"));
    }
    ensure_no_symlink_parents(root, FsPath::new(path))?;
    let target = root.join(path);
    match kind {
        ArtifactKind::File => {
            if std::fs::symlink_metadata(&target).is_ok() {
                return Err("input path already exists".into());
            }
            std::fs::create_dir_all(target.parent().ok_or("invalid input parent")?)
                .map_err(|error| error.to_string())?;
            std::fs::write(target, bytes).map_err(|error| error.to_string())
        }
        ArtifactKind::Tree => {
            if std::fs::symlink_metadata(&target).is_ok() {
                return Err("input tree path already exists".into());
            }
            extract_tar(&target, bytes)
        }
    }
}

fn extract_tar(target: &FsPath, reader: impl std::io::Read) -> Result<(), String> {
    const MAX_EXTRACTED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
    const MAX_ENTRIES: usize = 100_000;
    std::fs::create_dir_all(target).map_err(|error| error.to_string())?;
    let mut archive = tar::Archive::new(reader);
    let mut extracted = 0u64;
    for (index, entry) in archive
        .entries()
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if index >= MAX_ENTRIES {
            return Err("archive has too many entries".into());
        }
        let mut entry = entry.map_err(|error| error.to_string())?;
        extracted = extracted
            .checked_add(entry.size())
            .ok_or("archive size overflow")?;
        if extracted > MAX_EXTRACTED_BYTES {
            return Err("archive expands beyond 2 GiB".into());
        }
        let rel = entry
            .path()
            .map_err(|error| error.to_string())?
            .to_path_buf();
        if !fase_api::valid_path(rel.to_str().ok_or("non UTF-8 archive path")?) {
            return Err("unsafe archive path".into());
        }
        let destination = target.join(rel);
        let relative = destination
            .strip_prefix(target)
            .map_err(|e| e.to_string())?;
        ensure_no_symlink_parents(target, relative)?;
        match entry.header().entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(destination).map_err(|error| error.to_string())?
            }
            tar::EntryType::Regular => {
                if std::fs::symlink_metadata(&destination).is_ok() {
                    return Err("duplicate archive entry".into());
                }
                std::fs::create_dir_all(destination.parent().ok_or("invalid archive parent")?)
                    .map_err(|error| error.to_string())?;
                let mut file =
                    std::fs::File::create(destination).map_err(|error| error.to_string())?;
                std::io::copy(&mut entry, &mut file).map_err(|error| error.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mode =
                        if entry.header().mode().map_err(|error| error.to_string())? & 0o111 != 0 {
                            0o755
                        } else {
                            0o644
                        };
                    file.set_permissions(std::fs::Permissions::from_mode(mode))
                        .map_err(|error| error.to_string())?;
                }
            }
            tar::EntryType::Symlink => {
                let link = entry
                    .link_name()
                    .map_err(|e| e.to_string())?
                    .ok_or("missing symlink target")?;
                if !safe_link_target(relative, &link)
                    || std::fs::symlink_metadata(&destination).is_ok()
                {
                    return Err("unsafe symlink entry".into());
                }
                std::fs::create_dir_all(destination.parent().ok_or("invalid symlink parent")?)
                    .map_err(|e| e.to_string())?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(&link, &destination).map_err(|e| e.to_string())?;
                #[cfg(not(unix))]
                return Err("symlink extraction is unsupported on this platform".into());
            }
            _ => return Err("archive contains unsupported entry".into()),
        }
    }
    Ok(())
}

fn safe_link_target(link: &FsPath, target: &FsPath) -> bool {
    use std::path::Component;
    let mut depth = link.parent().map_or(0, |p| p.components().count());
    if target.is_absolute() || target.as_os_str().is_empty() {
        return false;
    }
    for part in target.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

fn ensure_no_symlink_parents(root: &FsPath, relative: &FsPath) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in relative
        .parent()
        .ok_or("invalid archive parent")?
        .components()
    {
        current.push(component.as_os_str());
        if std::fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("archive path traverses symlink".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn immutable_artifact_round_trip_verifies_content() {
        let store = Store {
            inner: Arc::new(object_store::memory::InMemory::new()),
            prefix: "test".into(),
        };
        let bytes = b"hello".to_vec();
        let name = artifact_id(&sha256_hex(&bytes), ArtifactKind::File);
        let spec = ArtifactSpec {
            content_digest: format!("sha256:{}", sha256_hex(&bytes)),
            size_bytes: bytes.len() as i64,
            kind: ArtifactKind::File,
            storage_ref: fase_api::StorageReference {
                key: format!("objects/{name}"),
            },
        };
        store.put(&name, bytes.clone(), u64::MAX).await.unwrap();
        assert_eq!(
            store
                .get(&name, &spec, ArtifactKind::File, u64::MAX)
                .await
                .unwrap(),
            bytes
        );
        assert!(
            store
                .get("art-wrong", &spec, ArtifactKind::File, u64::MAX)
                .await
                .is_err()
        );
    }

    #[test]
    fn artifact_identity_is_content_based_and_rejects_path_shapes() {
        let digest = sha256_hex(b"same");
        let file = artifact_id(&digest, ArtifactKind::File);
        let directory = artifact_id(&digest, ArtifactKind::Tree);
        assert_ne!(file, directory);
        assert!(valid_artifact_name(&file));
        assert!(!valid_artifact_name("art-../escape"));
        assert!(!valid_artifact_name("art-NOTHEX"));
    }

    #[test]
    fn tree_is_deterministic_and_preserves_executable_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let program = source.join("bin");
        std::fs::write(&program, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let first = collect(&source, ArtifactKind::Tree).unwrap();
        let second = collect(&source, ArtifactKind::Tree).unwrap();
        assert_eq!(first, second);
        let input_root = temp.path().join("input");
        materialize(&input_root, "tree", ArtifactKind::Tree, &first).unwrap();
        assert_eq!(
            std::fs::read(input_root.join("tree/bin")).unwrap(),
            b"#!/bin/sh\nexit 0\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(input_root.join("tree/bin"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn output_path_cannot_traverse_a_symlink() {
        let temp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc", temp.path().join("escape")).unwrap();
        assert!(checked_output_path(temp.path(), "escape/passwd").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn tree_preserves_safe_symlinks_and_rejects_escaping_links() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("data"), b"safe").unwrap();
        std::os::unix::fs::symlink("data", source.join("alias")).unwrap();
        let bytes = collect(&source, ArtifactKind::Tree).unwrap();
        materialize(temp.path(), "restored", ArtifactKind::Tree, &bytes).unwrap();
        assert_eq!(
            std::fs::read_link(temp.path().join("restored/alias")).unwrap(),
            std::path::Path::new("data")
        );
        std::os::unix::fs::symlink("../../etc/passwd", source.join("escape")).unwrap();
        assert!(collect(&source, ArtifactKind::Tree).is_err());
    }
}
