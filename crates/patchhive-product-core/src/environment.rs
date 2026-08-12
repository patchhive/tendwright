use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use anyhow::{Context, Result};

const EXPLICIT_ENV_FILE: &str = "PATCHHIVE_ENV_FILE";

fn private_file_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct PendingPrivateFile {
    path: PathBuf,
    armed: bool,
}

impl Drop for PendingPrivateFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Atomically update a secret-bearing text file without widening permissions.
///
/// The complete read/transform/write operation is serialized within the
/// process, symlink targets are refused, the fresh temporary file is created as
/// `0600`, and both file and parent directory are synced before returning.
pub fn update_private_text_file(
    path: &Path,
    update: impl FnOnce(&str) -> Result<String>,
) -> Result<()> {
    let _guard = private_file_write_lock()
        .lock()
        .map_err(|_| anyhow::anyhow!("private file update lock is poisoned"))?;

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("private file path has no file name: {}", path.display()))?;
    let lock_path = parent.join(format!("{}.lock", file_name.to_string_lossy()));
    let mut lock_options = OpenOptions::new();
    lock_options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock_file = lock_options
        .open(&lock_path)
        .with_context(|| format!("Could not open private file lock {}", lock_path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(
        &lock_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )?;
    fs2::FileExt::lock_exclusive(&lock_file)
        .with_context(|| format!("Could not lock private file {}", path.display()))?;

    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            anyhow::bail!(
                "refusing to replace non-regular private file {}",
                path.display()
            );
        }
    }
    let mut existing_options = OpenOptions::new();
    existing_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        existing_options.custom_flags(libc::O_NOFOLLOW);
    }
    let existing = match existing_options.open(path) {
        Ok(mut file) => {
            if !file
                .metadata()
                .with_context(|| format!("Could not inspect private file {}", path.display()))?
                .is_file()
            {
                anyhow::bail!(
                    "refusing to replace non-regular private file {}",
                    path.display()
                );
            }
            let mut content = String::new();
            file.read_to_string(&mut content)
                .with_context(|| format!("Could not read private file {}", path.display()))?;
            content
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Could not read private file {}", path.display()))
        }
    };
    let next = update(&existing)?;
    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut temp = options
        .open(&temp_path)
        .with_context(|| format!("Could not create private temp file {}", temp_path.display()))?;
    let mut pending = PendingPrivateFile {
        path: temp_path.clone(),
        armed: true,
    };
    temp.write_all(next.as_bytes())
        .with_context(|| format!("Could not write private temp file {}", temp_path.display()))?;
    temp.sync_all()
        .with_context(|| format!("Could not sync private temp file {}", temp_path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(
        &temp_path,
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )?;

    if fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
    {
        anyhow::bail!(
            "refusing to replace non-regular private file {}",
            path.display()
        );
    }
    fs::rename(&temp_path, path)
        .with_context(|| format!("Could not atomically replace {}", path.display()))?;
    pending.armed = false;
    #[cfg(unix)]
    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("Could not sync private file directory {}", parent.display()))?;
    fs2::FileExt::unlock(&lock_file)
        .with_context(|| format!("Could not unlock private file {}", path.display()))?;
    Ok(())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EnvironmentLoadReport {
    pub canonical_file: Option<PathBuf>,
    pub legacy_local_file: Option<PathBuf>,
}

/// Loads PatchHive configuration without making startup depend on the caller's
/// working directory.
///
/// Existing process variables always win. `PATCHHIVE_ENV_FILE` is authoritative
/// when present. In a monorepo checkout, the root `.env` is loaded next. A
/// product-local `.env` is then accepted only as a compatibility source for
/// values that were not supplied by the process or canonical file.
pub fn load_patchhive_env() -> Result<EnvironmentLoadReport> {
    if let Some(path) = nonempty_env(EXPLICIT_ENV_FILE).map(PathBuf::from) {
        load_required_env_file(&path)?;
        return Ok(EnvironmentLoadReport {
            canonical_file: Some(path),
            legacy_local_file: None,
        });
    }

    let current_dir = env::current_dir().context("Could not determine the current directory")?;
    let canonical_file = find_repo_root(&current_dir)
        .map(|root| root.join(".env"))
        .filter(|path| path.is_file());

    if let Some(path) = canonical_file.as_deref() {
        dotenvy::from_path(path).with_context(|| {
            format!("Could not load canonical PatchHive env {}", path.display())
        })?;
    }

    let local_file = current_dir.join(".env");
    let legacy_local_file = if local_file.is_file()
        && canonical_file
            .as_deref()
            .is_none_or(|canonical| !same_file(canonical, &local_file))
    {
        dotenvy::from_path(&local_file)
            .with_context(|| format!("Could not load legacy local env {}", local_file.display()))?;
        Some(local_file)
    } else {
        None
    };

    Ok(EnvironmentLoadReport {
        canonical_file,
        legacy_local_file,
    })
}

fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// The file PatchHive configuration should be *written* to.
///
/// Mirrors `load_patchhive_env`'s resolution so a value is persisted to the same
/// file it will later be read from: `PATCHHIVE_ENV_FILE` when set, otherwise the
/// monorepo root `.env`.
///
/// The fallback is deliberately the repo-root path even when that file does not
/// exist yet — creating the canonical file is correct, whereas defaulting to a
/// bare relative `.env` writes into whatever directory the process happens to be
/// started from. That is how a stray `services/patchhive-backend/.env` appeared:
/// the value was loaded from the root file and saved next to the binary, leaving
/// two secret stores when the canonical environment policy allows exactly one.
///
/// Returns `None` only outside a monorepo checkout with no explicit override, where
/// the caller's own default is the best available answer.
pub fn canonical_env_path() -> Option<PathBuf> {
    if let Some(path) = nonempty_env(EXPLICIT_ENV_FILE).map(PathBuf::from) {
        return Some(path);
    }
    let current_dir = env::current_dir().ok()?;
    find_repo_root(&current_dir).map(|root| root.join(".env"))
}

pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    start.ancestors().find_map(|candidate| {
        let has_repo_marker = candidate.join(".git").exists();
        let has_patchhive_marker =
            candidate.join("AGENTS.md").is_file() && candidate.join("products").is_dir();
        (has_repo_marker && has_patchhive_marker).then(|| candidate.to_path_buf())
    })
}

fn load_required_env_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        anyhow::bail!(
            "{EXPLICIT_ENV_FILE} points to missing file {}",
            path.display()
        );
    }
    dotenvy::from_path(path)
        .with_context(|| format!("Could not load {EXPLICIT_ENV_FILE} {}", path.display()))?;
    Ok(())
}

fn nonempty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{find_repo_root, update_private_text_file};
    use std::fs;

    #[test]
    fn repo_root_requires_git_and_patchhive_markers() {
        let base = std::env::temp_dir().join(format!("patchhive-env-root-{}", std::process::id()));
        let nested = base.join("products/example/backend");
        fs::create_dir_all(base.join(".git")).unwrap();
        fs::create_dir_all(base.join("products")).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::write(base.join("AGENTS.md"), "PatchHive").unwrap();

        assert_eq!(find_repo_root(&nested).as_deref(), Some(base.as_path()));

        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_updates_force_owner_only_permissions_and_reject_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let base =
            std::env::temp_dir().join(format!("patchhive-private-file-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&base).expect("create temp directory");
        let target = base.join(".env");
        fs::write(&target, "ONE=1\n").expect("write initial file");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
            .expect("set permissive mode");

        update_private_text_file(&target, |existing| Ok(format!("{existing}TWO=2\n")))
            .expect("update private file");
        assert_eq!(fs::read_to_string(&target).unwrap(), "ONE=1\nTWO=2\n");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let link = base.join("linked.env");
        symlink(&target, &link).expect("create symlink");
        assert!(update_private_text_file(&link, |_| Ok("SECRET=x\n".into())).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "ONE=1\nTWO=2\n");

        fs::remove_dir_all(base).expect("remove temp directory");
    }
}
