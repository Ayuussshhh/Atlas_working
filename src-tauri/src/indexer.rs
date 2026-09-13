use chrono::{DateTime, Local};
use rusqlite::{params, Connection, Result as SqlResult};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const LINES_PER_CHUNK: usize = 50;

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

#[derive(Debug, Clone)]
pub struct TextChunk {
    pub content: String,
    pub line_start: i64,
    pub line_end: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub snippet: String,
    pub line_start: i64,
    pub line_end: i64,
}

/// Extensions Atlas treats as readable text for indexing + search.
const TEXT_EXTENSIONS: &[&str] = &["txt", "md", "rs", "ts", "tsx", "js", "jsx", "py", "css"];

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| TEXT_EXTENSIONS.iter().any(|allowed| ext.eq_ignore_ascii_case(allowed)))
        .unwrap_or(false)
}

/// Walk a folder tree and collect full paths of text-like files.
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

/// Read file as UTF-8 text (lossy for odd bytes so indexing doesn't die).
pub fn read_text_file(path: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Split full text into line-ranged chunks for storage + search snippets.
pub fn chunk_text(text: &str) -> Vec<TextChunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + LINES_PER_CHUNK).min(lines.len());
        let content = lines[start..end].join("\n");
        if !content.trim().is_empty() {
            chunks.push(TextChunk {
                content,
                line_start: (start + 1) as i64,
                line_end: end as i64,
            });
        }
        start = end;
    }
    chunks
}

/// List → catalog → read → chunk → FTS for every text file under `root`.
pub fn index_folder(conn: &Connection, root: &str) -> Result<usize, String> {
    let paths = list_text_files(root)?;
    for path in &paths {
        let doc = path_to_document(path)?;
        let document_id = upsert_document(conn, &doc).map_err(|e| e.to_string())?;
        index_document_content(conn, document_id, &doc).map_err(|e| e.to_string())?;
    }
    Ok(paths.len())
}

fn index_document_content(
    conn: &Connection,
    document_id: i64,
    doc: &DocumentsType,
) -> SqlResult<()> {
    clear_document_chunks(conn, document_id, &doc.path)?;

    let text = match read_text_file(&doc.path) {
        Ok(t) => t,
        Err(_) => return Ok(()), // skip unreadable files; keep catalog row
    };

    for chunk in chunk_text(&text) {
        insert_chunk(conn, document_id, doc, &chunk)?;
    }
    Ok(())
}

fn clear_document_chunks(conn: &Connection, document_id: i64, path: &str) -> SqlResult<()> {
    conn.execute(
        "DELETE FROM chunks_fts WHERE path = ?1",
        params![path],
    )?;
    conn.execute(
        "DELETE FROM chunks WHERE document_id = ?1",
        params![document_id],
    )?;
    Ok(())
}

fn insert_chunk(
    conn: &Connection,
    document_id: i64,
    doc: &DocumentsType,
    chunk: &TextChunk,
) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO chunks (document_id, content, line_start, line_end)
         VALUES (?1, ?2, ?3, ?4)",
        params![document_id, chunk.content, chunk.line_start, chunk.line_end],
    )?;
    conn.execute(
        "INSERT INTO chunks_fts (content, path, name, line_start, line_end)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            chunk.content,
            doc.path,
            doc.name,
            chunk.line_start,
            chunk.line_end
        ],
    )?;
    Ok(())
}

/// Upsert catalog row and return its `documents.id`.
pub fn upsert_document(conn: &Connection, event: &DocumentsType) -> SqlResult<i64> {
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
    conn.query_row(
        "SELECT id FROM documents WHERE path = ?1",
        params![event.path],
        |row| row.get(0),
    )
}

/// Keep old name for callers that only need write-without-id.
pub fn insert_document(conn: &Connection, event: &DocumentsType) -> SqlResult<()> {
    upsert_document(conn, event).map(|_| ())
}

/// Read indexed rows back out.
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

/// Turn free text into a safe FTS5 MATCH query (token AND).
fn build_fts_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

/// Search remembered file contents via FTS5.
pub fn search(conn: &Connection, query: &str, limit: i64) -> SqlResult<Vec<SearchHit>> {
    let Some(fts_query) = build_fts_query(query) else {
        return Ok(Vec::new());
    };

    let mut stmt = conn.prepare(
        "SELECT path, name,
                snippet(chunks_fts, 0, '[', ']', '…', 14),
                line_start, line_end
         FROM chunks_fts
         WHERE chunks_fts MATCH ?1
         LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![fts_query, limit], |row| {
        Ok(SearchHit {
            path: row.get(0)?,
            name: row.get(1)?,
            snippet: row.get(2)?,
            line_start: row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            line_end: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
        })
    })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}
