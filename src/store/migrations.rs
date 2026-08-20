pub(crate) const BASE_SCHEMA_VERSION: u32 = 1;
pub(crate) const LATEST_SCHEMA_VERSION: u32 = 7;

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
        Migration {
            version: 5,
            name: "blocked_decision_context",
            apply: migrate_to_blocked_decision_context,
        },
        Migration {
            version: 6,
            name: "repair_blocked_decision_context",
            apply: migrate_to_blocked_decision_context,
        },
        Migration {
            version: 7,
            name: "jsonl_audit_cursor",
            apply: migrate_to_jsonl_audit,
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

fn migrate_to_blocked_decision_context(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), rusqlite::Error> {
    for (column_name, statement) in [
        (
            "blocker",
            "ALTER TABLE decision_requests ADD COLUMN blocker TEXT NOT NULL DEFAULT ''",
        ),
        (
            "options_json",
            "ALTER TABLE decision_requests ADD COLUMN options_json TEXT NOT NULL DEFAULT '[]'",
        ),
        (
            "recommendation",
            "ALTER TABLE decision_requests ADD COLUMN recommendation TEXT NOT NULL DEFAULT ''",
        ),
        (
            "resume_condition",
            "ALTER TABLE decision_requests ADD COLUMN resume_condition TEXT NOT NULL DEFAULT ''",
        ),
    ] {
        let has_column: bool = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM pragma_table_info('decision_requests')
                 WHERE name = ?1
             )",
            [column_name],
            |row| row.get(0),
        )?;
        if !has_column {
            transaction.execute(statement, [])?;
        }
    }
    Ok(())
}

fn migrate_to_jsonl_audit(transaction: &rusqlite::Transaction<'_>) -> Result<(), rusqlite::Error> {
    let has_audit_sequence: bool = transaction.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM pragma_table_info('audit_events')
             WHERE name = 'sequence'
         )",
        [],
        |row| row.get(0),
    )?;
    if !has_audit_sequence {
        transaction.execute("ALTER TABLE audit_events ADD COLUMN sequence INTEGER", [])?;
        transaction.execute(
            "UPDATE audit_events
             SET sequence = (
                 SELECT COUNT(*)
                 FROM audit_events AS previous
                 WHERE previous.rowid <= audit_events.rowid
             )",
            [],
        )?;
    }
    transaction.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS audit_events_sequence ON audit_events(sequence)",
        [],
    )?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS audit_jsonl_cursor (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             sequence INTEGER NOT NULL CHECK (sequence >= 0),
             previous_hash TEXT,
             updated_at TEXT NOT NULL
         );
         INSERT OR IGNORE INTO audit_jsonl_cursor (id, sequence, previous_hash, updated_at)
         VALUES (1, 0, NULL, datetime('now'));",
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
        assert!(super::is_supported_version(5));
        assert!(super::is_supported_version(6));
        assert!(super::is_supported_version(7));
        assert!(!super::is_supported_version(8));
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
                vec![3, 4, 5, 6, 7]
            );
            transaction.commit().unwrap();
        }

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            7
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
    fn schema_v4_v5_and_v6_add_and_repair_blocked_decision_context() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     applied_at TEXT NOT NULL
                 );
                 CREATE TABLE audit_events (id TEXT PRIMARY KEY);
                 CREATE TABLE decision_requests (
                     id TEXT PRIMARY KEY,
                     issue_id TEXT NOT NULL,
                     question TEXT NOT NULL,
                     background TEXT NOT NULL,
                     requester_kind TEXT,
                     requester_name TEXT,
                     requester_session_id TEXT,
                     status TEXT NOT NULL,
                     answer TEXT,
                     resolver_kind TEXT,
                     resolver_name TEXT,
                     resolver_session_id TEXT,
                     created_at TEXT NOT NULL,
                     resolved_at TEXT
                 );",
            )
            .unwrap();

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
            assert_eq!(
                applied.iter().map(|item| item.version).collect::<Vec<_>>(),
                vec![4, 5, 6, 7]
            );
            transaction.commit().unwrap();
        }

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            7
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
        for column in [
            "blocker",
            "options_json",
            "recommendation",
            "resume_condition",
        ] {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('decision_requests') WHERE name = ?1",
                        [column],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1,
                "missing decision request column {column}"
            );
        }
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
    fn schema_v6_repairs_decision_context_columns_missing_from_version_five() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 5).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     applied_at TEXT NOT NULL
                 );
                 CREATE TABLE decision_requests (
                     id TEXT PRIMARY KEY,
                     issue_id TEXT NOT NULL,
                     question TEXT NOT NULL,
                     background TEXT NOT NULL,
                     requester_kind TEXT,
                     requester_name TEXT,
                     requester_session_id TEXT,
                     status TEXT NOT NULL,
                     answer TEXT,
                     resolver_kind TEXT,
                     resolver_name TEXT,
                     resolver_session_id TEXT,
                     created_at TEXT NOT NULL,
                     resolved_at TEXT
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
                vec![6, 7]
            );
            transaction.commit().unwrap();
        }

        for column in [
            "blocker",
            "options_json",
            "recommendation",
            "resume_condition",
        ] {
            assert_eq!(
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM pragma_table_info('decision_requests') WHERE name = ?1",
                        [column],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1,
                "missing repaired decision request column {column}"
            );
        }
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            7
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
            assert_eq!(applied.len(), 6);
            transaction.commit().unwrap();
        }

        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
        assert!(applied.is_empty());
        transaction.commit().unwrap();
    }

    #[test]
    fn schema_v7_adds_jsonl_cursor() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 6).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     applied_at TEXT NOT NULL
                 );
                 CREATE TABLE audit_events (
                     id TEXT PRIMARY KEY,
                     occurred_at TEXT NOT NULL
                 );",
            )
            .unwrap();

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            let applied = super::apply_pending(&transaction, super::migrations()).unwrap();
            assert_eq!(
                applied.iter().map(|item| item.version).collect::<Vec<_>>(),
                vec![7]
            );
            transaction.commit().unwrap();
        }

        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            7
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('audit_events') WHERE name = 'sequence'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name = 'audit_jsonl_cursor'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        let cursor = connection
            .query_row(
                "SELECT id, sequence, previous_hash FROM audit_jsonl_cursor",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(cursor, (1, 0, None));
    }

    #[test]
    fn schema_v7_backfills_audit_sequence() {
        let mut connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 6).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     name TEXT NOT NULL,
                     applied_at TEXT NOT NULL
                 );
                 CREATE TABLE audit_events (
                     id TEXT PRIMARY KEY,
                     occurred_at TEXT NOT NULL
                 );
                 INSERT INTO audit_events (id, occurred_at) VALUES ('first', '2026-01-01T00:00:00Z');
                 INSERT INTO audit_events (id, occurred_at) VALUES ('second', '2026-01-01T00:00:01Z');",
            )
            .unwrap();

        {
            let transaction = connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            super::apply_pending(&transaction, super::migrations()).unwrap();
            transaction.commit().unwrap();
        }

        let mut statement = connection
            .prepare("SELECT id, sequence FROM audit_events ORDER BY sequence")
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![("first".to_owned(), 1), ("second".to_owned(), 2)]
        );
    }
}
