#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct BackupResult {
    pub(crate) output: std::path::PathBuf,
    pub(crate) schema_version: u32,
    pub(crate) format: &'static str,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct RestoreResult {
    pub(crate) output: std::path::PathBuf,
    pub(crate) schema_version: u32,
    pub(crate) audit_event_count: usize,
    pub(crate) format: &'static str,
}

pub(crate) fn create(
    source: &rusqlite::Connection,
    output: &std::path::Path,
) -> Result<BackupResult, crate::error::AppError> {
    if path_exists(output)? {
        return Err(crate::error::AppError::BackupOutputExists);
    }
    ensure_parent_directory(output)?;
    if has_sqlite_sidecar(output)? {
        return Err(crate::error::AppError::BackupDestinationInUse);
    }

    let temporary = create_temporary_file(output, "backup")?;
    let result = (|| {
        let mut destination = rusqlite::Connection::open_with_flags(
            &temporary,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|_| crate::error::AppError::BackupOperation {
            operation: "create",
        })?;
        destination
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| crate::error::AppError::BackupOperation {
                operation: "create",
            })?;

        {
            let backup = rusqlite::backup::Backup::new(source, &mut destination).map_err(|_| {
                crate::error::AppError::BackupOperation {
                    operation: "snapshot",
                }
            })?;
            backup
                .run_to_completion(100, std::time::Duration::from_millis(10), None)
                .map_err(|_| crate::error::AppError::BackupOperation {
                    operation: "snapshot",
                })?;
        }

        let schema_version = validate_connection(&destination)?;
        finalize_single_file(&destination)?;
        drop(destination);
        sync_file(&temporary)?;
        publish_new_file(&temporary, output)?;

        Ok(BackupResult {
            output: output.to_owned(),
            schema_version,
            format: "sqlite_online_backup",
        })
    })();

    if result.is_err() {
        remove_file_if_present(&temporary);
        remove_sqlite_sidecars(&temporary);
    }
    result
}

pub(crate) fn restore(
    input: &std::path::Path,
    output: &std::path::Path,
    replace: bool,
    confirmed: bool,
    context: &crate::domain::ExecutionContext,
    started_at: chrono::DateTime<chrono::Utc>,
) -> Result<RestoreResult, crate::error::AppError> {
    if !confirmed {
        return Err(crate::error::AppError::BackupConfirmationRequired);
    }
    if equivalent_paths(input, output) {
        return Err(crate::error::AppError::InvalidInput(
            "restore input and output must differ".to_owned(),
        ));
    }
    if has_sqlite_sidecar(input)? {
        return Err(crate::error::AppError::InvalidBackup(
            "backup sidecar files are not supported".to_owned(),
        ));
    }
    let source = open_backup_read_only(input)?;
    validate_connection(&source)?;
    validate_restore_destination(output, replace)?;
    ensure_parent_directory(output)?;

    let temporary = create_temporary_file(output, "restore")?;
    let temporary_jsonl = crate::store::jsonl::path_for_database(&temporary);
    let output_jsonl = crate::store::jsonl::path_for_database(output);
    let result = (|| {
        let mut destination = rusqlite::Connection::open_with_flags(
            &temporary,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
        .map_err(|_| crate::error::AppError::BackupOperation {
            operation: "restore",
        })?;
        destination
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| crate::error::AppError::BackupOperation {
                operation: "restore",
            })?;
        {
            let backup =
                rusqlite::backup::Backup::new(&source, &mut destination).map_err(|_| {
                    crate::error::AppError::BackupOperation {
                        operation: "restore",
                    }
                })?;
            backup
                .run_to_completion(100, std::time::Duration::from_millis(10), None)
                .map_err(|_| crate::error::AppError::BackupOperation {
                    operation: "restore",
                })?;
        }
        validate_connection(&destination)?;
        finalize_single_file(&destination)?;
        drop(destination);
        sync_file(&temporary)?;

        let mut database = crate::store::Database::open(&temporary)?;
        let schema_version = current_schema_version(database.connection())?;
        database.record_successful_operation(
            "restore",
            context,
            &crate::store::AuditSubject::default(),
            &[],
            started_at,
        )?;
        let audit = database.rebuild_audit_jsonl(&temporary_jsonl)?;
        finalize_single_file(database.connection())?;
        drop(database);
        remove_sqlite_sidecars(&temporary);
        sync_file(&temporary)?;
        publish_restore_files(&temporary, output, &temporary_jsonl, &output_jsonl, replace)?;

        Ok(RestoreResult {
            output: output.to_owned(),
            schema_version,
            audit_event_count: audit.event_count,
            format: "sqlite_online_backup",
        })
    })();

    if result.is_err() {
        remove_file_if_present(&temporary);
        remove_file_if_present(&temporary_jsonl);
        remove_sqlite_sidecars(&temporary);
    }
    result
}

fn open_backup_read_only(
    path: &std::path::Path,
) -> Result<rusqlite::Connection, crate::error::AppError> {
    if !path.is_file() {
        return Err(crate::error::AppError::InvalidBackup(
            "backup is not a regular file".to_owned(),
        ));
    }
    let connection =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| {
                crate::error::AppError::InvalidBackup(
                    "backup is not a valid SQLite database".to_owned(),
                )
            })?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|_| crate::error::AppError::InvalidBackup("backup cannot be read".to_owned()))?;
    Ok(connection)
}

fn current_schema_version(
    connection: &rusqlite::Connection,
) -> Result<u32, crate::error::AppError> {
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|_| crate::error::AppError::BackupOperation {
            operation: "restore",
        })?;
    u32::try_from(version).map_err(|_| crate::error::AppError::BackupOperation {
        operation: "restore",
    })
}

fn validate_restore_destination(
    output: &std::path::Path,
    replace: bool,
) -> Result<(), crate::error::AppError> {
    let output_exists = path_exists(output)?;
    if has_sqlite_sidecar(output)? {
        return Err(crate::error::AppError::BackupDestinationInUse);
    }
    if output_exists {
        let metadata = std::fs::symlink_metadata(output).map_err(|_| {
            crate::error::AppError::BackupOperation {
                operation: "output",
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(crate::error::AppError::BackupOutputExists);
        }
        if !replace {
            return Err(crate::error::AppError::BackupOutputExists);
        }
    }

    let output_jsonl = crate::store::jsonl::path_for_database(output);
    if path_exists(&output_jsonl)? && !replace {
        return Err(crate::error::AppError::BackupOutputExists);
    }
    Ok(())
}

fn equivalent_paths(left: &std::path::Path, right: &std::path::Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn validate_connection(connection: &rusqlite::Connection) -> Result<u32, crate::error::AppError> {
    let application_id = connection
        .pragma_query_value(None, "application_id", |row| row.get::<_, i64>(0))
        .map_err(|_| {
            crate::error::AppError::InvalidBackup("backup identity is unreadable".to_owned())
        })?;
    let application_id = u32::try_from(application_id).map_err(|_| {
        crate::error::AppError::InvalidBackup("backup identity is invalid".to_owned())
    })?;
    if application_id != crate::store::sqlite::BETTR_APPLICATION_ID {
        return Err(crate::error::AppError::DatabaseNotInitialized);
    }

    let schema_version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|_| {
            crate::error::AppError::InvalidBackup("backup schema version is unreadable".to_owned())
        })?;
    let schema_version = u32::try_from(schema_version).map_err(|_| {
        crate::error::AppError::InvalidBackup("backup schema version is invalid".to_owned())
    })?;
    if !crate::store::migrations::is_supported_version(schema_version) {
        return Err(crate::error::AppError::UnsupportedDatabaseSchemaVersion {
            found_version: schema_version,
            current_version: crate::store::migrations::LATEST_SCHEMA_VERSION,
        });
    }

    let integrity = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(|_| {
            crate::error::AppError::InvalidBackup("backup integrity check failed".to_owned())
        })?;
    if integrity != "ok" {
        return Err(crate::error::AppError::InvalidBackup(
            "backup integrity check failed".to_owned(),
        ));
    }

    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| {
            crate::error::AppError::InvalidBackup("backup foreign key check failed".to_owned())
        })?;
    let mut rows = statement.query([]).map_err(|_| {
        crate::error::AppError::InvalidBackup("backup foreign key check failed".to_owned())
    })?;
    if rows
        .next()
        .map_err(|_| {
            crate::error::AppError::InvalidBackup("backup foreign key check failed".to_owned())
        })?
        .is_some()
    {
        return Err(crate::error::AppError::InvalidBackup(
            "backup foreign key check failed".to_owned(),
        ));
    }

    Ok(schema_version)
}

fn ensure_parent_directory(path: &std::path::Path) -> Result<(), crate::error::AppError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    if parent.is_dir() {
        Ok(())
    } else {
        Err(crate::error::AppError::BackupOperation {
            operation: "output",
        })
    }
}

fn create_temporary_file(
    path: &std::path::Path,
    operation: &'static str,
) -> Result<std::path::PathBuf, crate::error::AppError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bettr-db");
    for _ in 0..8 {
        let temporary = parent.join(format!(".{stem}.{operation}-{}", uuid::Uuid::new_v4()));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(_) => return Ok(temporary),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                return Err(crate::error::AppError::BackupOperation { operation });
            }
        }
    }
    Err(crate::error::AppError::BackupOperation { operation })
}

fn path_exists(path: &std::path::Path) -> Result<bool, crate::error::AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(crate::error::AppError::BackupOperation {
            operation: "output",
        }),
    }
}

fn sqlite_sidecar(path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(suffix);
    sidecar.into()
}

fn has_sqlite_sidecar(path: &std::path::Path) -> Result<bool, crate::error::AppError> {
    ["-wal", "-shm", "-journal"]
        .iter()
        .try_fold(false, |found, suffix| {
            if found {
                return Ok(true);
            }
            path_exists(&sqlite_sidecar(path, suffix))
        })
}

fn remove_sqlite_sidecars(path: &std::path::Path) {
    for suffix in ["-wal", "-shm", "-journal"] {
        remove_file_if_present(&sqlite_sidecar(path, suffix));
    }
}

fn remove_file_if_present(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

fn finalize_single_file(connection: &rusqlite::Connection) -> Result<(), crate::error::AppError> {
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;")
        .map_err(|_| crate::error::AppError::BackupOperation {
            operation: "finalize",
        })
}

fn sync_file(path: &std::path::Path) -> Result<(), crate::error::AppError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| crate::error::AppError::BackupOperation { operation: "sync" })?;
    file.sync_data()
        .map_err(|_| crate::error::AppError::BackupOperation { operation: "sync" })
}

fn publish_new_file(
    temporary: &std::path::Path,
    output: &std::path::Path,
) -> Result<(), crate::error::AppError> {
    match std::fs::hard_link(temporary, output) {
        Ok(()) => {
            remove_file_if_present(temporary);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(crate::error::AppError::BackupOutputExists)
        }
        Err(_) => Err(crate::error::AppError::BackupOperation {
            operation: "publish",
        }),
    }
}

fn publish_restore_files(
    temporary: &std::path::Path,
    output: &std::path::Path,
    temporary_jsonl: &std::path::Path,
    output_jsonl: &std::path::Path,
    replace: bool,
) -> Result<(), crate::error::AppError> {
    if !replace {
        publish_new_file(temporary, output)?;
        if let Err(error) = publish_new_file(temporary_jsonl, output_jsonl) {
            remove_file_if_present(output);
            return Err(error);
        }
        return Ok(());
    }

    let old_output = if path_exists(output)? {
        Some(unique_path(output, "old-db"))
    } else {
        None
    };
    let old_jsonl = if path_exists(output_jsonl)? {
        Some(unique_path(output_jsonl, "old-jsonl"))
    } else {
        None
    };

    if let Some(old_output) = &old_output
        && std::fs::rename(output, old_output).is_err()
    {
        return Err(crate::error::AppError::BackupOperation {
            operation: "publish",
        });
    }
    if let Some(old_jsonl) = &old_jsonl
        && std::fs::rename(output_jsonl, old_jsonl).is_err()
    {
        if let Some(old_output) = &old_output {
            let _ = std::fs::rename(old_output, output);
        }
        return Err(crate::error::AppError::BackupOperation {
            operation: "publish",
        });
    }

    let publish_result = (|| {
        std::fs::rename(temporary, output).map_err(|_| {
            crate::error::AppError::BackupOperation {
                operation: "publish",
            }
        })?;
        std::fs::rename(temporary_jsonl, output_jsonl).map_err(|_| {
            crate::error::AppError::BackupOperation {
                operation: "publish",
            }
        })
    })();

    if let Err(error) = publish_result {
        remove_file_if_present(output);
        remove_file_if_present(output_jsonl);
        if let Some(old_output) = &old_output {
            let _ = std::fs::rename(old_output, output);
        }
        if let Some(old_jsonl) = &old_jsonl {
            let _ = std::fs::rename(old_jsonl, output_jsonl);
        }
        return Err(error);
    }

    if let Some(old_output) = &old_output {
        remove_file_if_present(old_output);
    }
    if let Some(old_jsonl) = &old_jsonl {
        remove_file_if_present(old_jsonl);
    }
    Ok(())
}

fn unique_path(path: &std::path::Path, operation: &str) -> std::path::PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bettr-db");
    parent.join(format!(".{stem}.{operation}-{}", uuid::Uuid::new_v4()))
}
