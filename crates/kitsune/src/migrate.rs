//! The store's schema, step by step: each migration goes up and comes back down, in one
//! transaction, and the version reached is kept in `schema_migrations`.

use crate::{Error, Result};
use rusqlite::{params, Connection};

struct Migration {
    version: u32,
    name: &'static str,
    up: &'static str,
    down: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "notes",
        up: include_str!("../migrations/0001_notes.up.sql"),
        down: include_str!("../migrations/0001_notes.down.sql"),
    },
    Migration {
        version: 2,
        name: "audio",
        up: include_str!("../migrations/0002_audio.up.sql"),
        down: include_str!("../migrations/0002_audio.down.sql"),
    },
    Migration {
        version: 3,
        name: "embeddings",
        up: include_str!("../migrations/0003_embeddings.up.sql"),
        down: include_str!("../migrations/0003_embeddings.down.sql"),
    },
    Migration {
        version: 4,
        name: "triggers",
        up: include_str!("../migrations/0004_triggers.up.sql"),
        down: include_str!("../migrations/0004_triggers.down.sql"),
    },
];

/// The newest schema this build knows.
pub const LATEST: u32 = 4;

fn ledger(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             name       TEXT NOT NULL,
             applied_at REAL NOT NULL
         );",
    )?;
    Ok(())
}

/// The schema's version now (0: empty).
pub fn version(conn: &Connection) -> Result<u32> {
    ledger(conn)?;
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )?)
}

/// Bring the schema to `target`, up or down, one migration per transaction.
pub fn migrate_to(conn: &mut Connection, target: u32) -> Result<u32> {
    if target > LATEST {
        return Err(Error::Invalid(format!(
            "schema {target} is newer than this build knows ({LATEST})"
        )));
    }
    let mut now = version(conn)?;
    if now > LATEST {
        return Err(Error::Invalid(format!(
            "this store has schema {now}, newer than this build ({LATEST}): update Inari"
        )));
    }
    while now < target {
        let m = &MIGRATIONS[now as usize];
        let tx = conn.transaction()?;
        tx.execute_batch(m.up)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            params![m.version, m.name, crate::store::now()],
        )?;
        tx.commit()?;
        now = m.version;
    }
    while now > target {
        let m = &MIGRATIONS[now as usize - 1];
        let tx = conn.transaction()?;
        tx.execute_batch(m.down)?;
        tx.execute(
            "DELETE FROM schema_migrations WHERE version = ?1",
            params![m.version],
        )?;
        tx.commit()?;
        now = m.version - 1;
    }
    Ok(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objects(conn: &Connection) -> Vec<String> {
        let mut st = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' \
                 AND name != 'schema_migrations' AND name NOT LIKE 'notes_fts_%' ORDER BY name",
            )
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
    }

    #[test]
    fn the_migrations_are_numbered_in_order() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1);
        }
        assert_eq!(MIGRATIONS.len() as u32, LATEST);
    }

    #[test]
    fn every_migration_goes_up_and_comes_back_down() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(version(&conn).unwrap(), 0);
        assert!(objects(&conn).is_empty());
        let mut seen = Vec::new();
        for v in 1..=LATEST {
            migrate_to(&mut conn, v).unwrap();
            assert_eq!(version(&conn).unwrap(), v);
            seen.push(objects(&conn));
        }
        assert!(seen.last().unwrap().contains(&"triggers".to_string()));
        // down one at a time: each step leaves exactly what the step below had
        for v in (0..LATEST).rev() {
            migrate_to(&mut conn, v).unwrap();
            let expected = if v == 0 {
                Vec::new()
            } else {
                seen[v as usize - 1].clone()
            };
            assert_eq!(objects(&conn), expected, "at schema {v}");
        }
        // and all the way up again
        migrate_to(&mut conn, LATEST).unwrap();
        assert_eq!(objects(&conn), *seen.last().unwrap());
    }

    #[test]
    fn a_schema_newer_than_the_build_is_refused() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert!(migrate_to(&mut conn, LATEST + 1).is_err());
        ledger(&conn).unwrap();
        conn.execute("INSERT INTO schema_migrations VALUES (99, 'future', 0)", [])
            .unwrap();
        assert!(migrate_to(&mut conn, LATEST).is_err());
    }
}
