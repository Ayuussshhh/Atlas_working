use sysinfo::{
    System, Disks,
};

use serde::Serialize;
#[derive(Serialize)]
// defined the structure and also that when going to IPC it must be converted to JSON
struct SystemInfo {
    cpu_percent: f32,
    used_memory: u64,
    total_memory: u64,
    used_disk: u64,
    total_disk: u64,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
    //.plugin(tauri_plugin_opener::init()) -> this is not being used and called here, it can open files/URLs in other apps
    .invoke_handler(tauri::generate_handler![get_system_info])
    .run(tauri::generate_context!())
    .expect("Error while running tauri application")
} 