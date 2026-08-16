#[cfg(test)]
mod tests {
    fn failing_migration(
        transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), rusqlite::Error> {
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
            assert!(super::apply_pending(&transaction, 1, &[migration]).is_err());
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
}
