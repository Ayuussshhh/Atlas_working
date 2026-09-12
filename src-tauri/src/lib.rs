use sysinfo::{Disks, System};
mod indexer;
mod activity;
mod database;
use activity::ActivityEvent;
use database::DB_PATH;
use indexer::DocumentsType;
use rusqlite::Connection;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};

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

/// Slice 1 prove: list text file paths under a root (no DB writes).
#[tauri::command]
fn debug_list_files() -> Result<Vec<String>, String> {
    // Relative to process cwd (often src-tauri when running tauri dev)
    let paths = indexer::list_text_files("../src")?;
    for p in &paths {
        println!("file: {p}");
    }
    Ok(paths)
}

/// Slice 2: walk `../src`, upsert metadata into `documents`, return count.
#[tauri::command]
fn index_files(db: State<'_, Arc<Mutex<Connection>>>) -> Result<usize, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let count = indexer::index_folder(&conn, "../src")?;
    println!("Indexed {count} documents");
    Ok(count)
}

#[tauri::command]
fn list_documents(db: State<'_, Arc<Mutex<Connection>>>) -> Result<Vec<DocumentsType>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    indexer::list_documents(&conn, 200).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let conn = database::init().expect("failed to initialize database");
            println!("Database ready at {}", DB_PATH);

            let db = Arc::new(Mutex::new(conn));
            app.manage(db.clone());
            activity::start_tracker(app.handle().clone(), db);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_system_info,
            get_processes_info,
            get_recent_activity,
            debug_list_files,
            index_files,
            list_documents
        ])
        .run(tauri::generate_context!())
        .expect("Error while running tauri application")
}
