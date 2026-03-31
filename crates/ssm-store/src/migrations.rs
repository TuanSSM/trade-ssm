use anyhow::Result;
use rusqlite::Connection;

/// A single schema migration.
struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

/// All migrations in order. New migrations are appended to this list.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_schema",
        sql: include_str!("../migrations/V001__initial_schema.sql"),
    },
    Migration {
        version: 2,
        name: "add_audit_log",
        sql: include_str!("../migrations/V002__add_audit_log.sql"),
    },
    Migration {
        version: 3,
        name: "add_dead_letters",
        sql: include_str!("../migrations/V003__add_dead_letters.sql"),
    },
];

/// Ensure the schema_migrations tracking table exists.
fn ensure_migrations_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version     INTEGER PRIMARY KEY,
            name        TEXT NOT NULL,
            applied_at  INTEGER NOT NULL DEFAULT (strftime('%s', 'now') * 1000)
        );",
    )?;
    Ok(())
}

/// Get the current schema version (0 if no migrations applied).
fn current_version(conn: &Connection) -> Result<u32> {
    let version: u32 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    Ok(version)
}

/// Run all pending migrations.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    ensure_migrations_table(conn)?;
    let current = current_version(conn)?;

    for migration in MIGRATIONS {
        if migration.version <= current {
            continue;
        }
        tracing::info!(
            version = migration.version,
            name = migration.name,
            "applying migration"
        );
        conn.execute_batch(migration.sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            rusqlite::params![migration.version, migration.name],
        )?;
    }

    let final_version = current_version(conn)?;
    if final_version > current {
        tracing::info!(from = current, to = final_version, "migrations complete");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn migrations_run_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();

        // Run migrations twice — second run should be a no-op
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let version = current_version(&conn).unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn migration_tracking_records_versions() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 3);
    }
}
