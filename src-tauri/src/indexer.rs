use chrono::{DateTime, Local};
use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const LINES_PER_CHUNK: usize = 40;
/// Office docs / PDFs can be large; still avoid multi-hundred-MB binaries.
const MAX_FILE_BYTES: u64 = 80_000_000;

/// Folders that are not user documents (build caches). Skipping these is why
/// Desktop/Documents PDFs actually get reached instead of drowning in node_modules.
const SKIP_TOOLING_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    "AppData",
    ".cargo",
    ".rustup",
    ".npm",
    ".cache",
    "dist",
    "build",
    ".next",
    "Coverage",
    "coverage",
    // Deep code trees on Desktop drown out real documents.
    "Programs",
    "Projects",
    "big_projects",
    ".windsurf",
    ".vscode",
    ".idea",
    "site-packages",
    "vendor",
    "migrations",
];

const INDEXABLE_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "rs", "ts", "tsx", "js", "jsx", "py", "css", "html",
    "htm", "json", "toml", "yaml", "yml", "csv", "log", "ini", "cfg", "conf",
    "xml", "sql", "sh", "bat", "ps1", "c", "cpp", "h", "hpp", "java", "go",
    "rb", "php", "r", "tex", "pdf", "docx", "pptx",
];

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
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub page_number: Option<i64>,
    pub slide_number: Option<i64>,
}

#[derive(Debug, Serialize, Clone)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub snippet: String,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub page_number: Option<i64>,
    pub slide_number: Option<i64>,
    pub file_type: String,
    pub location: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct FilePreview {
    pub path: String,
    pub name: String,
    pub file_type: String,
    pub size: i64,
    pub line_start: Option<i64>,
    pub line_end: Option<i64>,
    pub page_number: Option<i64>,
    pub slide_number: Option<i64>,
    pub location: String,
    pub content: String,
    pub query: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct IndexReport {
    pub indexed: usize,
    pub skipped: usize,
    pub scanned: usize,
    pub roots: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct IndexProgress {
    pub indexed: usize,
    pub skipped: usize,
    pub scanned: usize,
    pub current_path: String,
}

fn is_indexable_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            INDEXABLE_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
        .unwrap_or(false)
}

pub fn user_home() -> Result<PathBuf, String> {
    if let Ok(home) = std::env::var("USERPROFILE") {
        return Ok(PathBuf::from(home));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home));
    }
    Err("Could not resolve user home directory".into())
}

/// Prefer real user libraries (PDFs/docs live here), not the entire profile tree.
pub fn default_index_roots() -> Result<Vec<PathBuf>, String> {
    let home = user_home()?;
    let candidates = [
        home.join("Documents"),
        home.join("Desktop"),
        home.join("Downloads"),
        home.join("OneDrive"),
        home.join("OneDrive - Personal"),
    ];
    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in candidates {
        if path.is_dir() {
            let key = path.to_string_lossy().to_lowercase();
            if seen.insert(key) {
                roots.push(path);
            }
        }
    }
    if roots.is_empty() {
        if home.is_dir() {
            roots.push(home);
        } else {
            return Err("No indexable folders found".into());
        }
    }
    Ok(roots)
}

pub fn default_index_roots_display() -> Result<Vec<String>, String> {
    Ok(default_index_roots()?
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect())
}

pub fn list_text_files(root: &str) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    collect_files(Path::new(root), &mut paths, 0)?;
    Ok(paths)
}

fn collect_files(dir: &Path, out: &mut Vec<String>, depth: usize) -> Result<(), String> {
    if depth > 40 {
        return Ok(());
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if SKIP_TOOLING_DIRS
                    .iter()
                    .any(|s| name.eq_ignore_ascii_case(s))
                {
                    continue;
                }
            }
            collect_files(&path, out, depth + 1)?;
        } else if path.is_file() && is_indexable_file(&path) {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.eq_ignore_ascii_case("__init__.py")
                    || name.eq_ignore_ascii_case(".DS_Store")
                {
                    continue;
                }
            }
            if let Ok(meta) = path.metadata() {
                if meta.len() > MAX_FILE_BYTES {
                    continue;
                }
                // Skip empty stubs that create noise in the index.
                if meta.len() < 3 {
                    continue;
                }
            }
            out.push(path.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

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
    let modified_secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Fast fingerprint: extractor version + size + mtime — bump version to force re-extract.
    let hash = format!("v2:{}:{}", data.len(), modified_secs);

    Ok(DocumentsType {
        path: path.to_string(),
        name,
        file_type,
        size: data.len() as i64,
        modified_at: DateTime::<Local>::from(modified).to_rfc3339(),
        indexed_at: Local::now().to_rfc3339(),
        hash,
    })
}

fn strip_xml_to_text(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut in_tag = false;
    let mut last_space = true;
    for ch in xml.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
            _ if !in_tag => {
                if ch.is_whitespace() {
                    if !last_space {
                        out.push(' ');
                        last_space = true;
                    }
                } else {
                    out.push(ch);
                    last_space = false;
                }
            }
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn chunk_plain_text(text: &str) -> Vec<TextChunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        if text.trim().is_empty() {
            return Vec::new();
        }
        return vec![TextChunk {
            content: text.to_string(),
            line_start: Some(1),
            line_end: Some(1),
            page_number: None,
            slide_number: None,
        }];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + LINES_PER_CHUNK).min(lines.len());
        let content = lines[start..end].join("\n");
        if !content.trim().is_empty() {
            chunks.push(TextChunk {
                content,
                line_start: Some((start + 1) as i64),
                line_end: Some(end as i64),
                page_number: None,
                slide_number: None,
            });
        }
        start = end;
    }
    chunks
}

fn extract_pdf_chunks(path: &str) -> Result<Vec<TextChunk>, String> {
    let bytes = fs::read(path).map_err(|e| format!("pdf read {path}: {e}"))?;
    if bytes.len() < 5 || !bytes.starts_with(b"%PDF") {
        return Err(format!("pdf {path}: not a PDF (bad header)"));
    }

    if let Ok(pages) = pdf_extract::extract_text_from_mem_by_pages(&bytes) {
        let mut out = Vec::new();
        for (i, page) in pages.into_iter().enumerate() {
            let content = page.trim().to_string();
            if content.is_empty() {
                continue;
            }
            out.push(TextChunk {
                content,
                line_start: None,
                line_end: None,
                page_number: Some((i + 1) as i64),
                slide_number: None,
            });
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }

    let text = match pdf_extract::extract_text_from_mem(&bytes) {
        Ok(t) if !t.trim().is_empty() => t,
        _ => salvage_pdf_strings(&bytes),
    };

    if text.trim().is_empty() {
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        return Ok(vec![TextChunk {
            content: format!(
                "{name} (PDF has little extractable text — may be scanned or image-based)"
            ),
            line_start: None,
            line_end: None,
            page_number: Some(1),
            slide_number: None,
        }]);
    }

    let pages: Vec<&str> = text
        .split('\u{000C}')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if pages.is_empty() {
        return Ok(chunk_plain_text(&text));
    }
    let mut out = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        out.push(TextChunk {
            content: (*page).to_string(),
            line_start: None,
            line_end: None,
            page_number: Some((i + 1) as i64),
            slide_number: None,
        });
    }
    Ok(out)
}

/// Pull readable `(...)` Tj / TJ strings when pdf-extract fails (corrupt trailer, etc.).
fn salvage_pdf_strings(bytes: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut chars = lossy.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '(' {
            continue;
        }
        let mut piece = String::new();
        let mut depth = 1i32;
        let mut escaped = false;
        while let Some(c) = chars.next() {
            if escaped {
                match c {
                    'n' => piece.push('\n'),
                    'r' => piece.push('\r'),
                    't' => piece.push('\t'),
                    _ => piece.push(c),
                }
                escaped = false;
                continue;
            }
            match c {
                '\\' => escaped = true,
                '(' => {
                    depth += 1;
                    piece.push(c);
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    piece.push(c);
                }
                _ => piece.push(c),
            }
        }
        let trimmed = piece.trim();
        if trimmed.len() < 2 {
            continue;
        }
        // Skip binary-ish garbage.
        let printable = trimmed
            .chars()
            .filter(|c| c.is_ascii_graphic() || c.is_whitespace())
            .count();
        if printable * 10 < trimmed.chars().count() * 7 {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(trimmed);
        if out.len() > 80_000 {
            break;
        }
    }
    out
}

fn extract_docx_chunks(path: &str) -> Result<Vec<TextChunk>, String> {
    let file = fs::File::open(path).map_err(|e| format!("docx open {path}: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("docx zip {path}: {e}"))?;

    let mut combined = String::new();
    let mut names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let item = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = item.name().to_string();
        if name == "word/document.xml"
            || name.starts_with("word/header")
            || name.starts_with("word/footer")
            || name.starts_with("word/footnotes")
            || name.starts_with("word/endnotes")
        {
            names.push(name);
        }
    }
    names.sort();
    for name in names {
        let mut part = archive.by_name(&name).map_err(|e| e.to_string())?;
        let mut xml = String::new();
        part.read_to_string(&mut xml)
            .map_err(|e| format!("docx read {path}: {e}"))?;
        let with_breaks = xml
            .replace("</w:p>", "</w:p>\n")
            .replace("</a:t>", "</a:t> ");
        let text = strip_xml_to_text(&with_breaks);
        if !text.trim().is_empty() {
            if !combined.is_empty() {
                combined.push_str("\n\n");
            }
            combined.push_str(&text);
        }
    }

    if combined.trim().is_empty() {
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        return Ok(vec![TextChunk {
            content: format!(
                "{name} (Word file appears image-only — open the file to view)"
            ),
            line_start: Some(1),
            line_end: Some(1),
            page_number: None,
            slide_number: None,
        }]);
    }
    Ok(chunk_plain_text(&combined))
}

fn extract_pptx_chunks(path: &str) -> Result<Vec<TextChunk>, String> {
    let file = fs::File::open(path).map_err(|e| format!("pptx open {path}: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("pptx zip {path}: {e}"))?;
    let mut slide_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let item = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = item.name().to_string();
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
            slide_names.push(name);
        }
    }
    slide_names.sort_by(|a, b| {
        let num = |s: &str| {
            s.trim_start_matches("ppt/slides/slide")
                .trim_end_matches(".xml")
                .parse::<u32>()
                .unwrap_or(0)
        };
        num(a).cmp(&num(b))
    });

    let mut out = Vec::new();
    for (i, name) in slide_names.iter().enumerate() {
        let mut slide = archive.by_name(name).map_err(|e| e.to_string())?;
        let mut xml = String::new();
        slide.read_to_string(&mut xml).map_err(|e| e.to_string())?;
        let text = strip_xml_to_text(&xml);
        if text.trim().is_empty() {
            continue;
        }
        out.push(TextChunk {
            content: text,
            line_start: None,
            line_end: None,
            page_number: None,
            slide_number: Some((i + 1) as i64),
        });
    }
    if out.is_empty() {
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        out.push(TextChunk {
            content: format!(
                "{name} (presentation has little extractable text — open to view)"
            ),
            line_start: None,
            line_end: None,
            page_number: None,
            slide_number: Some(1),
        });
    }
    Ok(out)
}

/// Build location-aware chunks for any supported file type.
pub fn extract_chunks(path: &str) -> Result<Vec<TextChunk>, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => extract_pdf_chunks(path),
        "docx" => extract_docx_chunks(path),
        "pptx" => extract_pptx_chunks(path),
        "ppt" => Err("Legacy .ppt is not supported — save as .pptx".into()),
        _ => {
            let bytes = fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
            Ok(chunk_plain_text(&String::from_utf8_lossy(&bytes)))
        }
    }
}

pub fn read_document_text(path: &str) -> Result<String, String> {
    let chunks = extract_chunks(path)?;
    Ok(chunks
        .into_iter()
        .map(|c| c.content)
        .collect::<Vec<_>>()
        .join("\n\n"))
}

pub fn index_folder(conn: &Connection, root: &str) -> Result<usize, String> {
    let paths = list_text_files(root)?;
    for path in &paths {
        let doc = path_to_document(path)?;
        let document_id = upsert_document(conn, &doc).map_err(|e| e.to_string())?;
        index_document_content(conn, document_id, &doc).map_err(|e| e.to_string())?;
    }
    Ok(paths.len())
}

pub fn index_machine_with_progress<F>(
    conn: &Connection,
    mut on_progress: F,
) -> Result<IndexReport, String>
where
    F: FnMut(IndexProgress),
{
    let roots = default_index_roots()?;
    let mut indexed = 0usize;
    let mut skipped = 0usize;
    let mut scanned = 0usize;
    let mut seen = std::collections::HashSet::new();

    for root in &roots {
        let root_str = root.to_string_lossy();
        println!("Scanning root: {root_str}");
        let paths = list_text_files(&root_str)?;
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            scanned += 1;
            let doc = match path_to_document(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };

            // Skip unchanged files (same size + mtime fingerprint).
            if let Ok(Some(old_hash)) = stored_hash(conn, &doc.path) {
                if old_hash == doc.hash {
                    skipped += 1;
                    if scanned % 25 == 0 {
                        on_progress(IndexProgress {
                            indexed,
                            skipped,
                            scanned,
                            current_path: path.clone(),
                        });
                    }
                    continue;
                }
            }

            let document_id = match upsert_document(conn, &doc) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("upsert failed {}: {e}", doc.path);
                    continue;
                }
            };
            match index_document_content(conn, document_id, &doc) {
                Ok(chunk_count) => {
                    let is_doc = matches!(
                        doc.file_type.to_ascii_lowercase().as_str(),
                        "pdf" | "docx" | "pptx"
                    );
                    if chunk_count == 0 && is_doc {
                        eprintln!("no searchable text extracted: {}", doc.path);
                    }
                    indexed += 1;
                }
                Err(e) => eprintln!("chunk index failed {}: {e}", doc.path),
            }
            if indexed % 3 == 0 || indexed == 1 {
                on_progress(IndexProgress {
                    indexed,
                    skipped,
                    scanned,
                    current_path: path.clone(),
                });
            }
        }
    }

    Ok(IndexReport {
        indexed,
        skipped,
        scanned,
        roots: roots
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    })
}

pub fn index_machine(conn: &Connection) -> Result<IndexReport, String> {
    index_machine_with_progress(conn, |_| {})
}

fn stored_hash(conn: &Connection, path: &str) -> SqlResult<Option<String>> {
    let value: Option<Option<String>> = conn
        .query_row(
            "SELECT hash FROM documents WHERE path = ?1",
            params![path],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    Ok(value.flatten().filter(|h| !h.is_empty()))
}

fn index_document_content(
    conn: &Connection,
    document_id: i64,
    doc: &DocumentsType,
) -> SqlResult<usize> {
    clear_document_chunks(conn, document_id, &doc.path)?;
    let chunks = match extract_chunks(&doc.path) {
        Ok(c) => c,
        Err(e) => {
            let is_doc = matches!(
                doc.file_type.to_ascii_lowercase().as_str(),
                "pdf" | "docx" | "pptx"
            );
            if is_doc {
                eprintln!("extract failed {}: {e}", doc.path);
            }
            // Still remember the file for filename search + preview message.
            vec![TextChunk {
                content: format!(
                    "{} (could not read content: {})",
                    doc.name,
                    e.chars().take(120).collect::<String>()
                ),
                line_start: None,
                line_end: None,
                page_number: if doc.file_type.eq_ignore_ascii_case("pdf") {
                    Some(1)
                } else {
                    None
                },
                slide_number: None,
            }]
        }
    };
    let count = chunks.len();
    for chunk in &chunks {
        insert_chunk(conn, document_id, doc, chunk)?;
    }
    Ok(count)
}

fn clear_document_chunks(conn: &Connection, document_id: i64, path: &str) -> SqlResult<()> {
    conn.execute("DELETE FROM chunks_fts WHERE path = ?1", params![path])?;
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
        "INSERT INTO chunks (document_id, content, page_number, slide_number, line_start, line_end)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            document_id,
            chunk.content,
            chunk.page_number,
            chunk.slide_number,
            chunk.line_start,
            chunk.line_end
        ],
    )?;
    conn.execute(
        "INSERT INTO chunks_fts (content, path, name, line_start, line_end, page_number, slide_number, file_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            chunk.content,
            doc.path,
            doc.name,
            chunk.line_start,
            chunk.line_end,
            chunk.page_number,
            chunk.slide_number,
            doc.file_type
        ],
    )?;
    Ok(())
}

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

fn build_fts_query(raw: &str) -> Option<String> {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
        .collect();
    if tokens.is_empty() {
        None
    } else {
        // Phrase-friendly: AND tokens so "bit manipulation" prefers chunks with both.
        Some(tokens.join(" "))
    }
}

fn hit_location(page: Option<i64>, slide: Option<i64>, start: Option<i64>, end: Option<i64>) -> String {
    if let Some(p) = page {
        return format!("Page {p}");
    }
    if let Some(s) = slide {
        return format!("Slide {s}");
    }
    match (start, end) {
        (Some(a), Some(b)) if a == b => format!("Line {a}"),
        (Some(a), Some(b)) => format!("Lines {a}–{b}"),
        _ => "Match".into(),
    }
}

pub fn search(conn: &Connection, query: &str, limit: i64) -> SqlResult<Vec<SearchHit>> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Some(fts_query) = build_fts_query(query) {
        let mut stmt = conn.prepare(
            "SELECT path, name,
                    snippet(chunks_fts, 0, '⟦', '⟧', '…', 18),
                    line_start, line_end, page_number, slide_number, file_type
             FROM chunks_fts
             WHERE chunks_fts MATCH ?1
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![fts_query, limit], |row| {
            let line_start: Option<i64> = row.get(3)?;
            let line_end: Option<i64> = row.get(4)?;
            let page_number: Option<i64> = row.get(5)?;
            let slide_number: Option<i64> = row.get(6)?;
            let file_type: String = row.get::<_, Option<String>>(7)?.unwrap_or_default();
            Ok(SearchHit {
                path: row.get(0)?,
                name: row.get(1)?,
                snippet: row.get(2)?,
                line_start,
                line_end,
                page_number,
                slide_number,
                file_type,
                location: hit_location(page_number, slide_number, line_start, line_end),
            })
        })?;

        for row in rows {
            let hit = row?;
            let key = format!(
                "{}:{}:{}:{}",
                hit.path,
                hit.page_number.unwrap_or(0),
                hit.slide_number.unwrap_or(0),
                hit.line_start.unwrap_or(0)
            );
            if seen.insert(key) {
                out.push(hit);
            }
        }
    }

    // Filename matches are secondary — content chunks come first (PRD).
    let like = format!("%{}%", query.trim());
    let mut name_stmt = conn.prepare(
        "SELECT path, name, type
         FROM documents
         WHERE name LIKE ?1 OR path LIKE ?1
         ORDER BY name
         LIMIT ?2",
    )?;
    let name_rows = name_stmt.query_map(params![like, limit], |row| {
        let file_type: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
        Ok(SearchHit {
            path: row.get(0)?,
            name: row.get(1)?,
            snippet: "Matched by filename (open to browse)".into(),
            line_start: None,
            line_end: None,
            page_number: None,
            slide_number: None,
            file_type,
            location: "Filename".into(),
        })
    })?;

    for row in name_rows {
        let hit = row?;
        if seen.insert(format!("{}:name", hit.path)) {
            out.push(hit);
        }
        if out.len() as i64 >= limit {
            break;
        }
    }

    Ok(out)
}

pub fn file_preview(
    conn: &Connection,
    path: &str,
    line_start: Option<i64>,
    line_end: Option<i64>,
    page_number: Option<i64>,
    slide_number: Option<i64>,
    query: &str,
) -> Result<FilePreview, String> {
    // Prefer indexed chunks — never re-parse PDF/DOCX on every Spotlight keystroke.
    let (name, file_type, size) = conn
        .query_row(
            "SELECT name, type, size FROM documents WHERE path = ?1",
            params![path],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                ))
            },
        )
        .unwrap_or_else(|_| {
            let fallback = path_to_document(path).unwrap_or(DocumentsType {
                path: path.to_string(),
                name: Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string(),
                file_type: Path::new(path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string(),
                size: 0,
                modified_at: String::new(),
                indexed_at: String::new(),
                hash: String::new(),
            });
            (fallback.name, fallback.file_type, fallback.size)
        });

    let chunks = load_preview_chunks(conn, path).unwrap_or_default();
    let ext = file_type.to_ascii_lowercase();
    let is_officeish = matches!(ext.as_str(), "pdf" | "docx" | "pptx" | "doc" | "txt" | "md");

    let (content, location) = if let Some(page) = page_number {
        let body = chunks
            .iter()
            .find(|c| c.page_number == Some(page))
            .map(|c| c.content.clone())
            .or_else(|| chunks.first().map(|c| c.content.clone()))
            .unwrap_or_default();
        (trim_preview(&body, 2800), format!("Page {page}"))
    } else if let Some(slide) = slide_number {
        let body = chunks
            .iter()
            .find(|c| c.slide_number == Some(slide))
            .map(|c| c.content.clone())
            .unwrap_or_default();
        (trim_preview(&body, 2800), format!("Slide {slide}"))
    } else if is_officeish || line_start.is_none() {
        let body = chunks
            .iter()
            .take(3)
            .map(|c| c.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let loc = match ext.as_str() {
            "pdf" => "Document".into(),
            "pptx" => "Presentation".into(),
            "docx" | "doc" => "Document".into(),
            _ => hit_location(None, None, line_start, line_end),
        };
        (trim_preview(&body, 2800), loc)
    } else {
        let text = chunks
            .iter()
            .map(|c| c.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let lines: Vec<&str> = text.lines().collect();
        let body = if lines.is_empty() {
            text
        } else {
            let start = (line_start.unwrap_or(1).max(1) as usize).saturating_sub(1);
            let end = (line_end.unwrap_or(start as i64 + 1).max(1) as usize).min(lines.len());
            let pad_start = start.saturating_sub(4);
            let pad_end = (end + 8).min(lines.len());
            lines[pad_start..pad_end]
                .iter()
                .enumerate()
                .map(|(i, line)| format!("{:>4} │ {}", pad_start + i + 1, line))
                .collect::<Vec<_>>()
                .join("\n")
        };
        (
            trim_preview(&body, 2800),
            hit_location(page_number, slide_number, line_start, line_end),
        )
    };

    let content = if content.trim().is_empty() {
        "No indexed preview yet — run Update index, or open the file.".into()
    } else {
        content
    };

    Ok(FilePreview {
        path: path.to_string(),
        name,
        file_type,
        size,
        line_start,
        line_end,
        page_number,
        slide_number,
        location,
        content,
        query: query.to_string(),
    })
}

fn load_preview_chunks(conn: &Connection, path: &str) -> SqlResult<Vec<TextChunk>> {
    let mut stmt = conn.prepare(
        "SELECT c.content, c.page_number, c.slide_number, c.line_start, c.line_end
         FROM chunks c
         JOIN documents d ON d.id = c.document_id
         WHERE d.path = ?1
         ORDER BY c.id
         LIMIT 12",
    )?;
    let rows = stmt.query_map(params![path], |row| {
        Ok(TextChunk {
            content: row.get(0)?,
            page_number: row.get(1)?,
            slide_number: row.get(2)?,
            line_start: row.get(3)?,
            line_end: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn trim_preview(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    format!("{}…", trimmed.chars().take(max_chars).collect::<String>())
}

/// Open a file; for PDFs try to jump near the page when the OS viewer supports it.
pub fn open_document(path: &str, page_number: Option<i64>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Some(page) = page_number {
            let target = format!("{path}#page={page}");
            let _ = Command::new("cmd")
                .creation_flags(CREATE_NO_WINDOW)
                .args(["/C", "start", "", &target])
                .status();
            return Ok(());
        }
        Command::new("cmd")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["/C", "start", "", path])
            .status()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (path, page_number);
        Err("open_document: unsupported OS".into())
    }
}
