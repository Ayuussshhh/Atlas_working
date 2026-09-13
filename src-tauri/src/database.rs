use rusqlite::{Connection, Result};
use std::path::Path;

pub const DB_PATH: &str = "database/atlas.db";

pub fn open_connection(path: &str) -> Result<Connection> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).expect("failed to create database directory");
        }
    }
    let conn = Connection::open(path)?;
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "busy_timeout", 5000);
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS documents (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            type TEXT,
            size INTEGER,
            modified_at TEXT,
            indexed_at TEXT,
            hash TEXT
        );

        CREATE TABLE IF NOT EXISTS chunks (
            id INTEGER PRIMARY KEY,
            document_id INTEGER NOT NULL,
            content TEXT NOT NULL,
            page_number INTEGER,
            slide_number INTEGER,
            line_start INTEGER,
            line_end INTEGER,
            FOREIGN KEY (document_id) REFERENCES documents(id)
        );

        CREATE TABLE IF NOT EXISTS activity_events (
            id INTEGER PRIMARY KEY,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            application TEXT,
            window_title TEXT,
            path TEXT,
            event_type TEXT
        );

        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;

    let version: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key = 'fts_version'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "0".into());

    if version != "3" {
        conn.execute_batch(
            "
            DROP TABLE IF EXISTS chunks_fts;
            CREATE VIRTUAL TABLE chunks_fts USING fts5(
                content,
                path UNINDEXED,
                name UNINDEXED,
                line_start UNINDEXED,
                line_end UNINDEXED,
                page_number UNINDEXED,
                slide_number UNINDEXED,
                file_type UNINDEXED,
                tokenize = 'porter unicode61'
            );
            ",
        )?;
        conn.execute(
            "INSERT INTO schema_meta(key, value) VALUES('fts_version', '3')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
    }

    Ok(())
}

pub fn init() -> Result<Connection> {
    let conn = open_connection(DB_PATH)?;
    migrate(&conn)?;
    Ok(conn)
}
