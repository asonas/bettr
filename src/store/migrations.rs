pub(crate) const BASE_SCHEMA_VERSION: u32 = 1;
pub(crate) const LATEST_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy)]
pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) name: &'static str,
    pub(crate) apply: fn(&rusqlite::Transaction<'_>) -> Result<(), rusqlite::Error>,
}

pub(crate) fn is_supported_version(version: u32) -> bool {
    (BASE_SCHEMA_VERSION..=LATEST_SCHEMA_VERSION).contains(&version)
}

pub(crate) fn migrations() -> &'static [Migration] {
    &[Migration {
        version: LATEST_SCHEMA_VERSION,
        name: "schema_migrations",
        apply: migrate_to_schema_migrations,
    }]
}

pub(crate) fn apply_pending(
    transaction: &rusqlite::Transaction<'_>,
    migrations: &[Migration],
) -> Result<Vec<Migration>, rusqlite::Error> {
    let current_version: i64 =
        transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let mut applied = Vec::new();
    for migration in migrations
        .iter()
        .filter(|migration| i64::from(migration.version) > current_version)
    {
        (migration.apply)(transaction)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![
                migration.version,
                migration.name,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        transaction.pragma_update(None, "user_version", migration.version)?;
        applied.push(*migration);
    }
    Ok(applied)
}

fn migrate_to_schema_migrations(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at TEXT NOT NULL
         );",
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO schema_migrations (version, name, applied_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            BASE_SCHEMA_VERSION,
            "phase1_baseline",
            chrono::Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    fn failing_migration(transaction: &rusqlite::Transaction<'_>) -> Result<(), rusqlite::Error> {
        transaction.execute_batch(
            "CREATE TABLE rollback_target (value TEXT NOT NULL);\n\
             INSERT INTO rollback_target VALUES ('must disappear');",
        )?;
        Err(rusqlite::Error::InvalidQuery)
    }

    #[test]
    fn failed_migration_can_be_rolled_back_without_history_or_version_change() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        let migration = super::Migration {
            version: 2,
            name: "failing migration",
            apply: failing_migration,
        };

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            assert!(super::apply_pending(&transaction, &[migration]).is_err());
        }

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name = 'rollback_target'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn only_known_schema_versions_are_supported() {
        assert!(!super::is_supported_version(0));
        assert!(super::is_supported_version(1));
        assert!(super::is_supported_version(2));
        assert!(!super::is_supported_version(3));
    }

    #[test]
    fn a_second_transaction_rechecks_the_schema_version_after_another_migration_commits() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
            assert_eq!(applied.len(), 1);
            transaction.commit().unwrap();
        }

        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
        assert!(applied.is_empty());
        transaction.commit().unwrap();
    }
}
