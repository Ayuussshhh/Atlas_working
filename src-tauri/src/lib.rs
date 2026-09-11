use sysinfo::{
    System, Disks,
};
mod activity;
mod database;
use activity::ActivityEvent;
use database::DB_PATH;
use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Manager, State};

#[derive(Serialize)]
// defined the structure and also that when going to IPC it must be converted to JSON
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
    // we are using this to find the memory(RAM), disk and cpu usage
    let mut sys = System::new_all();

    // disks used variable initialized 
    let mut total_space = 0;
    let mut available_space = 0;
    let mut used_space = 0;

    // after initializing the variables we are using this to find the disks usage and all
    sys.refresh_all();
    let disks = Disks::new_with_refreshed_list();
    for disks in &disks{
        if disks.mount_point() == "C:\\" {
            total_space = disks.total_space();
            available_space = disks.available_space();
            used_space = total_space - available_space;
        }
        else{
            println!("Error while reading the sysinfo");
        }
    }

    // cpu refreshing and all and also fetching the global cpu working
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
        let row = ProcessInfo {
            process_name: process.name().to_string_lossy().into_owned(),
            cpu_usage: process.cpu_usage(),
            memory_used: process.memory(),
            process_id: pid.as_u32(),
        };
        list.push(row);
    }
    list
}

/// Read focused window from Windows, INSERT into activity_events, return the event to React.
/// React never opens SQLite — it only receives the result.
#[tauri::command]
fn record_active_window(db: State<'_, Mutex<Connection>>) -> Result<ActivityEvent, String> {
    let event = activity::get_active_window()?;
    println!(
        "Active window: {} | {}",
        event.application, event.window_title
    );

    let conn = db.lock().map_err(|e| e.to_string())?;
    activity::insert_activity_event(&conn, &event).map_err(|e| e.to_string())?;

    Ok(event)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
    .setup(|app| {
        // DB is owned by Rust at boot — React never opens or migrates it.
        let conn = database::init().expect("failed to initialize database");
        println!("Database ready at {}", DB_PATH);
        app.manage(Mutex::new(conn));
        Ok(())
    })
    .invoke_handler(tauri::generate_handler![
        get_system_info,
        get_processes_info,
        record_active_window
    ])
    .run(tauri::generate_context!())
    .expect("Error while running tauri application")
}
