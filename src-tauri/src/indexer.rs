use chrono::{DateTime, Local};
use rusqlite::{params, Connection, Result as SqlResult};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Clone)]
pub struct DocumentsType {
    pub path: String,
    pub name: String,
    pub file_type: String,
    pub size: i64,
    pub modified_at: String,
    pub indexed_at: String,
    pub hash: String,
}

/// Extensions Atlas treats as text for Slice 1 (list only — no DB yet).
const TEXT_EXTENSIONS: &[&str] = &["txt", "md", "rs", "ts", "tsx", "js", "jsx", "py", "css"];

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| TEXT_EXTENSIONS.iter().any(|allowed| ext.eq_ignore_ascii_case(allowed)))
        .unwrap_or(false)
}

/// Walk a folder tree and collect full paths of text-like files.
/// Same shape as listing processes: many entries → loop → keep some.
pub fn list_text_files(root: &str) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    collect_text_files(Path::new(root), &mut paths)?;
    Ok(paths)
}

fn collect_text_files(dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path: PathBuf = entry.path();

        if path.is_dir() {
            // Go deeper into subfolders (skip target/node_modules noise later if needed)
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "target" || name == "node_modules" || name == ".git" {
                    continue;
                }
            }
            collect_text_files(&path, out)?;
        } else if path.is_file() && is_text_file(&path) {
            out.push(path.to_string_lossy().into_owned());
        }
    }

    Ok(())
}

/// One path string → one filled document mold (disk metadata, not DB).
pub fn path_to_document(path: &str) -> Result<DocumentsType, String> {
    let data = fs::metadata(path).map_err(|e| e.to_string())?;
    let p = Path::new(path);

    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();

    let file_type = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();

    let modified = data.modified().map_err(|e| e.to_string())?;
    let modified_at = DateTime::<Local>::from(modified).to_rfc3339();

    Ok(DocumentsType {
        path: path.to_string(),
        name,
        file_type,
        size: data.len() as i64,
        modified_at,
        indexed_at: Local::now().to_rfc3339(),
        hash: String::new(),
    })
}

/// List → enrich → upsert for every text file under `root`.
/// Returns how many documents were written.
pub fn index_folder(conn: &Connection, root: &str) -> Result<usize, String> {
    let paths = list_text_files(root)?;
    for path in &paths {
        let doc = path_to_document(path)?;
        insert_document(conn, &doc).map_err(|e| e.to_string())?;
    }
    Ok(paths.len())
}

pub fn insert_document(conn: &Connection, event: &DocumentsType) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO documents (path, name, type, size, modified_at, indexed_at, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(path) DO UPDATE SET
           name = excluded.name,
           type = excluded.type,
           size = excluded.size,
           modified_at = excluded.modified_at,
           indexed_at = excluded.indexed_at,
           hash = excluded.hash",
        params![
            event.path,
            event.name,
            event.file_type,
            event.size,
            event.modified_at,
            event.indexed_at,
            event.hash,
        ],
    )?;
    Ok(())
}

/// Read indexed rows back out (prove Slice 2 in the UI / DB Browser).
pub fn list_documents(conn: &Connection, limit: i64) -> SqlResult<Vec<DocumentsType>> {
    let mut stmt = conn.prepare(
        "SELECT path, name, type, size, modified_at, indexed_at, hash
         FROM documents
         ORDER BY indexed_at DESC
         LIMIT ?1",
    )?;

    let rows = stmt.query_map(params![limit], |row| {
        Ok(DocumentsType {
            path: row.get(0)?,
            name: row.get(1)?,
            file_type: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            size: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            modified_at: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            indexed_at: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            hash: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        })
    })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}
