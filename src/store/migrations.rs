pub(crate) const BASE_SCHEMA_VERSION: u32 = 1;
pub(crate) const LATEST_SCHEMA_VERSION: u32 = 4;

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
    &[
        Migration {
            version: 2,
            name: "schema_migrations",
            apply: migrate_to_schema_migrations,
        },
        Migration {
            version: 3,
            name: "phase_two_coordination",
            apply: migrate_to_phase_two_coordination,
        },
        Migration {
            version: 4,
            name: "idempotency_and_audit",
            apply: migrate_to_idempotency,
        },
    ]
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

fn migrate_to_phase_two_coordination(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS issue_dependencies (
             id TEXT PRIMARY KEY,
             blocker_issue_id TEXT NOT NULL REFERENCES issues(id),
             blocked_issue_id TEXT NOT NULL REFERENCES issues(id),
             relation TEXT NOT NULL CHECK (relation = 'blocks'),
             created_at TEXT NOT NULL,
             UNIQUE(blocker_issue_id, blocked_issue_id, relation),
             CHECK (blocker_issue_id <> blocked_issue_id)
         );
         CREATE INDEX IF NOT EXISTS issue_dependencies_blocker
             ON issue_dependencies(blocker_issue_id);
         CREATE INDEX IF NOT EXISTS issue_dependencies_blocked
             ON issue_dependencies(blocked_issue_id);
         CREATE TABLE IF NOT EXISTS issue_parents (
             child_issue_id TEXT PRIMARY KEY REFERENCES issues(id),
             parent_issue_id TEXT NOT NULL REFERENCES issues(id),
             created_at TEXT NOT NULL,
             CHECK (child_issue_id <> parent_issue_id)
         );
         CREATE INDEX IF NOT EXISTS issue_parents_parent
             ON issue_parents(parent_issue_id);
         CREATE TABLE IF NOT EXISTS issue_leases (
             issue_id TEXT PRIMARY KEY REFERENCES issues(id),
             agent TEXT NOT NULL,
             session_id TEXT NOT NULL,
             claimed_at TEXT NOT NULL,
             heartbeat_at TEXT NOT NULL,
             expires_at TEXT NOT NULL,
             lease_revision INTEGER NOT NULL CHECK (lease_revision > 0)
         );
         CREATE INDEX IF NOT EXISTS issue_leases_expires_at
             ON issue_leases(expires_at);
         CREATE TABLE IF NOT EXISTS decision_requests (
             id TEXT PRIMARY KEY,
             issue_id TEXT NOT NULL REFERENCES issues(id),
             question TEXT NOT NULL,
             background TEXT NOT NULL,
             requester_kind TEXT,
             requester_name TEXT,
             requester_session_id TEXT,
             status TEXT NOT NULL CHECK (status IN ('open', 'resolved')),
             answer TEXT,
             resolver_kind TEXT,
             resolver_name TEXT,
             resolver_session_id TEXT,
             created_at TEXT NOT NULL,
             resolved_at TEXT
         );
         CREATE INDEX IF NOT EXISTS decision_requests_issue_status
             ON decision_requests(issue_id, status);
         CREATE INDEX IF NOT EXISTS decision_requests_status
             ON decision_requests(status);",
    )
}

fn migrate_to_idempotency(transaction: &rusqlite::Transaction<'_>) -> Result<(), rusqlite::Error> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS idempotency_records (
             idempotency_key TEXT PRIMARY KEY,
             operation TEXT NOT NULL,
             request_hash TEXT NOT NULL,
             response_json TEXT NOT NULL,
             created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idempotency_records_created_at
             ON idempotency_records(created_at);",
    )?;
    let has_audit_idempotency_key: bool = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM pragma_table_info('audit_events')
             WHERE name = 'idempotency_key'
         )",
        [],
        |row| row.get(0),
    )?;
    if !has_audit_idempotency_key {
        transaction.execute(
            "ALTER TABLE audit_events ADD COLUMN idempotency_key TEXT",
            [],
        )?;
    }
    transaction.execute(
        "CREATE INDEX IF NOT EXISTS audit_events_idempotency_key
         ON audit_events(idempotency_key)",
        [],
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
        assert!(super::is_supported_version(3));
        assert!(super::is_supported_version(4));
        assert!(!super::is_supported_version(5));
    }

    #[test]
    fn schema_v3_adds_phase_two_coordination_tables() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 2).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     applied_at TEXT NOT NULL
                 );
                 CREATE TABLE audit_events (id TEXT PRIMARY KEY);",
            )
            .unwrap();

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
            assert_eq!(
                applied.iter().map(|item| item.version).collect::<Vec<_>>(),
                vec![3, 4]
            );
            transaction.commit().unwrap();
        }

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            4
        );
        for table in [
            "issue_dependencies",
            "issue_parents",
            "issue_leases",
            "decision_requests",
        ] {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                        [table],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1,
                "missing table {table}"
            );
        }
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'idempotency_records'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn schema_v4_adds_idempotency_records_and_audit_key() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     applied_at TEXT NOT NULL
                 );
                 CREATE TABLE audit_events (id TEXT PRIMARY KEY);",
            )
            .unwrap();

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
            assert_eq!(
                applied.iter().map(|item| item.version).collect::<Vec<_>>(),
                vec![4]
            );
            transaction.commit().unwrap();
        }

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            4
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'idempotency_records'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('audit_events') WHERE name = 'idempotency_key'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn a_second_transaction_rechecks_the_schema_version_after_another_migration_commits() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        connection
            .execute("CREATE TABLE audit_events (id TEXT PRIMARY KEY)", [])
            .unwrap();

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
            assert_eq!(applied.len(), 3);
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
