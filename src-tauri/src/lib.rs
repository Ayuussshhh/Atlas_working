use sysinfo::{Disks, System};
mod activity;
mod database;
mod indexer;
use activity::ActivityEvent;
use database::DB_PATH;
use indexer::{DocumentsType, FilePreview, IndexProgress, IndexReport, SearchHit};
use rusqlite::Connection;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

struct IndexingFlag(AtomicBool);

#[derive(Serialize)]
struct SystemInfo {
    cpu_percent: f32,
    used_memory: u64,
    total_memory: u64,
    used_disk: u64,
    total_disk: u64,
}

#[derive(Serialize)]
struct ProcessInfo {
    process_name: String,
    cpu_usage: f32,
    memory_used: u64,
    process_id: u32,
}

fn show_spotlight_only(app: &tauri::AppHandle) {
    // Wispr-style: only the overlay window — do NOT raise the main dashboard.
    if let Some(win) = app.get_webview_window("spotlight") {
        let _ = win.show();
        let _ = win.set_focus();
        let _ = win.emit("spotlight-shown", ());
    }
}

fn hide_spotlight_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("spotlight") {
        let _ = win.hide();
    }
}

#[tauri::command]
fn get_system_info() -> SystemInfo {
    let mut sys = System::new_all();

    let mut total_space = 0u64;
    let mut used_space = 0u64;

    sys.refresh_all();
    let disks = Disks::new_with_refreshed_list();
    for disk in &disks {
        if disk.mount_point() == std::path::Path::new("C:\\") {
            total_space = disk.total_space();
            let available = disk.available_space();
            used_space = total_space.saturating_sub(available);
        }
    }

    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    let cpu_working = sys.global_cpu_usage();

    SystemInfo {
        cpu_percent: cpu_working,
        used_memory: sys.used_memory(),
        total_memory: sys.total_memory(),
        used_disk: used_space,
        total_disk: total_space,
    }
}

#[tauri::command]
fn get_processes_info() -> Vec<ProcessInfo> {
    let mut list = Vec::new();
    let mut sys2 = System::new_all();
    sys2.refresh_all();
    sys2.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys2.refresh_cpu_usage();

    for (pid, process) in sys2.processes() {
        list.push(ProcessInfo {
            process_name: process.name().to_string_lossy().into_owned(),
            cpu_usage: process.cpu_usage(),
            memory_used: process.memory(),
            process_id: pid.as_u32(),
        });
    }

    list.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    list.truncate(15);
    list
}

#[tauri::command]
fn get_recent_activity(db: State<'_, Arc<Mutex<Connection>>>) -> Result<Vec<ActivityEvent>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    activity::recent_activity(&conn, 12).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_index_roots() -> Result<Vec<String>, String> {
    indexer::default_index_roots_display()
}

#[tauri::command]
fn debug_list_files() -> Result<Vec<String>, String> {
    let roots = indexer::default_index_roots()?;
    let mut all = Vec::new();
    for root in roots {
        let mut paths = indexer::list_text_files(&root.to_string_lossy())?;
        all.append(&mut paths);
        if all.len() > 200 {
            all.truncate(200);
            break;
        }
    }
    for p in &all {
        println!("file: {p}");
    }
    Ok(all)
}

#[derive(Serialize, Clone)]
struct IndexDoneEvent {
    ok: bool,
    indexed: usize,
    skipped: usize,
    scanned: usize,
    roots: Vec<String>,
    error: Option<String>,
}

fn spawn_index(app: tauri::AppHandle, flag: Arc<IndexingFlag>) -> Result<(), String> {
    if flag
        .0
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Indexing is already running".into());
    }

    std::thread::spawn(move || {
        let result = (|| -> Result<IndexReport, String> {
            let conn = database::open_connection(DB_PATH).map_err(|e| e.to_string())?;
            database::migrate(&conn).map_err(|e| e.to_string())?;
            indexer::index_machine_with_progress(&conn, |progress: IndexProgress| {
                let _ = app.emit("index-progress", progress);
            })
        })();

        match result {
            Ok(report) => {
                println!(
                    "Index update: {} new/changed, {} unchanged, {} scanned across {} roots",
                    report.indexed,
                    report.skipped,
                    report.scanned,
                    report.roots.len()
                );
                let _ = app.emit(
                    "index-done",
                    IndexDoneEvent {
                        ok: true,
                        indexed: report.indexed,
                        skipped: report.skipped,
                        scanned: report.scanned,
                        roots: report.roots,
                        error: None,
                    },
                );
            }
            Err(err) => {
                eprintln!("Index failed: {err}");
                let _ = app.emit(
                    "index-done",
                    IndexDoneEvent {
                        ok: false,
                        indexed: 0,
                        skipped: 0,
                        scanned: 0,
                        roots: vec![],
                        error: Some(err),
                    },
                );
            }
        }

        flag.0.store(false, Ordering::SeqCst);
    });

    Ok(())
}

/// Start machine indexing on a background thread — does not freeze the UI.
#[tauri::command]
fn index_files(
    app: tauri::AppHandle,
    flag: State<'_, Arc<IndexingFlag>>,
) -> Result<(), String> {
    spawn_index(app, flag.inner().clone())
}

#[tauri::command]
fn list_documents(db: State<'_, Arc<Mutex<Connection>>>) -> Result<Vec<DocumentsType>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    indexer::list_documents(&conn, 400).map_err(|e| e.to_string())
}

#[tauri::command]
fn search_files(query: String) -> Result<Vec<SearchHit>, String> {
    // Own connection so Spotlight never waits on the activity tracker mutex.
    let conn = database::open_connection(DB_PATH).map_err(|e| e.to_string())?;
    indexer::search(&conn, &query, 40).map_err(|e| e.to_string())
}

#[tauri::command]
fn preview_file(
    path: String,
    line_start: Option<i64>,
    line_end: Option<i64>,
    page_number: Option<i64>,
    slide_number: Option<i64>,
    query: Option<String>,
) -> Result<FilePreview, String> {
    let conn = database::open_connection(DB_PATH).map_err(|e| e.to_string())?;
    indexer::file_preview(
        &conn,
        &path,
        line_start,
        line_end,
        page_number,
        slide_number,
        query.as_deref().unwrap_or(""),
    )
}

#[tauri::command]
fn open_path(
    app: tauri::AppHandle,
    path: String,
    page_number: Option<i64>,
) -> Result<(), String> {
    if page_number.is_some() {
        if indexer::open_document(&path, page_number).is_ok() {
            return Ok(());
        }
    }
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_spotlight_window(app: tauri::AppHandle) {
    show_spotlight_only(&app);
}

#[tauri::command]
fn hide_spotlight(app: tauri::AppHandle) {
    hide_spotlight_window(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::new(IndexingFlag(AtomicBool::new(false))))
        .setup(|app| {
            let conn = database::init().expect("failed to initialize database");
            println!("Database ready at {}", DB_PATH);

            let db = Arc::new(Mutex::new(conn));
            app.manage(db.clone());
            activity::start_tracker(app.handle().clone(), db);

            // Do not auto-index on every launch. Index only when the user clicks
            // Update index (incremental hash skip still applies then).

            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::{
                    Builder, GlobalShortcutExt, ShortcutState,
                };

                // Build plugin without pre-registering shortcuts (avoids hard panic
                // when a hotkey is already taken by Windows / another app).
                app.handle().plugin(
                    Builder::new()
                        .with_handler(|app, _shortcut, event| {
                            if event.state == ShortcutState::Pressed {
                                show_spotlight_only(app);
                            }
                        })
                        .build(),
                )?;

                // Prefer Wispr-like Ctrl+Win+Space; fall back if already registered.
                let candidates = [
                    "ctrl+super+space",
                    "ctrl+shift+space",
                    "alt+space",
                ];
                let mut registered_any = false;
                for shortcut in candidates {
                    match app.global_shortcut().register(shortcut) {
                        Ok(()) => {
                            println!("Spotlight shortcut registered: {shortcut}");
                            registered_any = true;
                            break;
                        }
                        Err(err) => {
                            eprintln!("Spotlight shortcut skipped ({shortcut}): {err}");
                        }
                    }
                }
                if !registered_any {
                    eprintln!(
                        "No global Spotlight shortcut could be registered. Use the Spotlight button in Atlas."
                    );
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_system_info,
            get_processes_info,
            get_recent_activity,
            get_index_roots,
            debug_list_files,
            index_files,
            list_documents,
            search_files,
            preview_file,
            open_path,
            open_spotlight_window,
            hide_spotlight
        ])
        .run(tauri::generate_context!())
        .expect("Error while running tauri application")
}
