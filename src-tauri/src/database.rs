use rusqlite::{Connection, Result};
use std::path::Path;

pub const DB_PATH: &str = "database/atlas.db";

/// Open (or create) the SQLite file at `path`.
/// Creates the parent folder if needed.
pub fn open_connection(path: &str) -> Result<Connection> {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).expect("failed to create database directory");
        }
    }
    Connection::open(path)
}

/// Create Atlas tables if they do not exist yet.
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
        ",
    )?;
    Ok(())
}

/// Called once at app startup from Rust — React never does this.
pub fn init() -> Result<Connection> {
    let conn = open_connection(DB_PATH)?;
    migrate(&conn)?;
    Ok(conn)
}
