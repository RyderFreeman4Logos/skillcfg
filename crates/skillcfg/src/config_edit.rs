use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
    process::{Command, Stdio},
};

use rustix::{
    fs::{FlockOperation, Mode, OFlags, fgetxattr_query, flock, lgetxattr_query, open},
    io::Errno,
};
use skillcfg_core::{Config, ConfigError, display_path};
use tempfile::{Builder, NamedTempFile};

use crate::CliError;

const ACCESS_ACL: &str = "system.posix_acl_access";

pub(crate) enum Operation<'a> {
    Edit {
        program: &'a OsStr,
        editor_args: &'a [OsString],
    },
    Restore,
}

pub(crate) fn run(path: &Path, operation: Operation<'_>) -> Result<(), CliError> {
    // Resolve the regular target so replacing it does not break a config symlink.
    let target = fs::canonicalize(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(CliError::Failure(
            "config must be a regular file".to_owned(),
        ));
    };
    let mut backup_name = name.to_os_string();
    backup_name.push(".prev");
    let backup = parent.join(backup_name);

    // The directory remains the same inode when a config or its backup is replaced.
    let directory = File::open(parent).map_err(|e| failure("open config directory", parent, e))?;
    flock(&directory, FlockOperation::LockExclusive)
        .map_err(|e| failure("lock config directory", parent, e.into()))?;
    let (original, metadata) = read_regular(&target)?;
    require_mode_only(lgetxattr_query(&target, ACCESS_ACL), path)?;
    if metadata.nlink() != 1 {
        return Err(CliError::Failure(
            "cannot atomically edit a config with hard links".to_owned(),
        ));
    }
    let scratch = Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in(parent)
        .map_err(|e| failure("create private edit directory", parent, e))?;
    let (edited, check_path) = match operation {
        Operation::Edit {
            program,
            editor_args,
        } => {
            let candidate = scratch.path().join("candidate.toml");
            let editor_file = staged(scratch.path(), &original)?;
            editor_file
                .persist(&candidate)
                .map_err(|e| failure("stage editor candidate", path, e.error))?;
            let status = Command::new(program)
                .args(editor_args.iter().map(|arg| {
                    if arg == "{}" {
                        candidate.as_os_str()
                    } else {
                        arg.as_os_str()
                    }
                }))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|e| failure("run config editor", path, e))?;
            if !status.success() {
                return Err(CliError::Failure(format!(
                    "config editor failed ({status})"
                )));
            }
            (read_regular(&candidate)?.0, path)
        }
        Operation::Restore => (read_regular(&backup)?.0, backup.as_path()),
    };
    let source = std::str::from_utf8(&edited).map_err(|error| ConfigError::Read {
        path: check_path.to_owned(),
        source: io::Error::new(io::ErrorKind::InvalidData, error),
    })?;
    Config::parse(source).map_err(|source| ConfigError::AtPath {
        path: check_path.to_owned(),
        source: Box::new(source),
    })?;
    if edited == original {
        return Ok(());
    }

    let replacement = staged(scratch.path(), &edited)?;
    let new_metadata = replacement
        .as_file()
        .metadata()
        .map_err(|e| failure("check replacement owner", path, e))?;
    if new_metadata.uid() != metadata.uid() || new_metadata.gid() != metadata.gid() {
        return Err(CliError::Failure(
            "replacing the config would change its owner or group".to_owned(),
        ));
    }
    replacement
        .as_file()
        .set_permissions(metadata.permissions())
        .map_err(|e| failure("preserve config permissions", path, e))?;
    require_mode_only(fgetxattr_query(replacement.as_file(), ACCESS_ACL), path)?;
    replacement
        .as_file()
        .sync_all()
        .map_err(|e| failure("sync replacement config", path, e))?;
    let previous = staged(scratch.path(), &original)?;
    require_mode_only(fgetxattr_query(previous.as_file(), ACCESS_ACL), &backup)?;
    previous
        .as_file()
        .sync_all()
        .map_err(|e| failure("sync previous config", &backup, e))?;
    let retained_backup = scratch.path().join("original-prev");
    let had_backup = match fs::symlink_metadata(&backup) {
        Ok(info) if !info.is_file() => {
            return Err(CliError::Failure(format!(
                "previous config '{}' must be a regular file",
                display_path(&backup)
            )));
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(failure("inspect previous config", &backup, error));
        }
        Ok(_) => {
            fs::hard_link(&backup, &retained_backup)
                .map_err(|e| failure("retain previous config", &backup, e))?;
            true
        }
        Err(_) => false,
    };

    let (current, current_metadata) = read_regular(&target)?;
    require_mode_only(lgetxattr_query(&target, ACCESS_ACL), path)?;
    if fs::canonicalize(path).ok().as_ref() != Some(&target)
        || current != original
        || unchanged_attributes(&metadata) != unchanged_attributes(&current_metadata)
    {
        return Err(CliError::Failure(
            "config changed during the edit; retry from the current file".to_owned(),
        ));
    }

    previous
        .persist(&backup)
        .map_err(|e| failure("install previous config", &backup, e.error))?;
    // shortcut: an abrupt interruption between the two renames can leave the
    // prior backup in the private edit directory. Use a journal for auto-recovery.
    if let Err(error) = directory.sync_all().and_then(|()| {
        replacement
            .persist(&target)
            .map(|_| ())
            .map_err(|e| e.error)
    }) {
        let rollback = if had_backup {
            fs::rename(&retained_backup, &backup)
        } else {
            fs::remove_file(&backup)
        };
        if let Err(rollback_error) = rollback {
            let recovery = scratch.keep();
            return Err(CliError::Failure(format!(
                "cannot install validated config: {error}; cannot roll back previous config: {rollback_error}; recovery directory '{}'",
                display_path(&recovery)
            )));
        }
        if let Err(sync_error) = directory.sync_all() {
            eprintln!(
                "skillcfg: restored the prior backup, but could not sync its directory: {sync_error}"
            );
        }
        return Err(failure("install validated config", path, error));
    }
    if let Err(error) = directory.sync_all() {
        eprintln!("skillcfg: updated config, but could not sync its directory: {error}");
    }
    Ok(())
}

fn require_mode_only(acl: rustix::io::Result<usize>, path: &Path) -> Result<(), CliError> {
    match acl {
        Err(Errno::NODATA | Errno::NOTSUP) => Ok(()),
        Ok(_) => Err(CliError::Failure(format!(
            "cannot preserve access ACL for config '{}'",
            display_path(path)
        ))),
        Err(error) => Err(failure("inspect config access ACL", path, error.into())),
    }
}

fn unchanged_attributes(metadata: &fs::Metadata) -> (u64, u64, u64, u32, u32, u32) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.nlink(),
        metadata.uid(),
        metadata.gid(),
        metadata.mode(),
    )
}

fn read_regular(path: &Path) -> Result<(Vec<u8>, fs::Metadata), ConfigError> {
    let read = || -> io::Result<_> {
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let mut file = File::from(open(path, flags, Mode::empty())?);
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "config must be a regular file",
            ));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok((bytes, metadata))
    };
    read().map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })
}

fn staged(directory: &Path, bytes: &[u8]) -> Result<NamedTempFile, CliError> {
    let mut file = NamedTempFile::new_in(directory)
        .map_err(|error| failure("create private config copy", directory, error))?;
    file.write_all(bytes)
        .map_err(|error| failure("write private config copy", directory, error))?;
    Ok(file)
}

fn failure(action: &str, path: &Path, error: io::Error) -> CliError {
    CliError::Failure(format!("cannot {action} '{}': {error}", display_path(path)))
}
