use active_win_pos_rs::get_active_window as os_get_active_window;
use chrono::Local;
use rusqlite::{params, Connection, Result as SqlResult};
use serde::Serialize;

/// One activity row — same shape as activity_events columns (mailbox for OS → DB → React).
#[derive(Debug, Serialize, Clone)]
pub struct ActivityEvent {
    pub started_at: String,
    pub ended_at: String,
    pub application: String,
    pub window_title: String,
    pub path: String,
    pub event_type: String,
}

/// Ask Windows which window has focus. Does NOT touch SQLite.
pub fn get_active_window() -> Result<ActivityEvent, String> {
    let win = os_get_active_window().map_err(|_| {
        "Could not read the foreground window (none focused, or OS denied access)".to_string()
    })?;

    // active-win-pos-rs already did: foreground HWND → title + process name + exe path
    Ok(ActivityEvent {
        started_at: Local::now().to_rfc3339(),
        ended_at: String::new(),
        application: win.app_name,
        window_title: win.title,
        path: win.process_path.to_string_lossy().into_owned(),
        event_type: "active_window".to_string(),
    })
}

/// Write one event into activity_events. Does NOT talk to Windows.
pub fn insert_activity_event(conn: &Connection, event: &ActivityEvent) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO activity_events
            (started_at, ended_at, application, window_title, path, event_type)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            event.started_at,
            event.ended_at,
            event.application,
            event.window_title,
            event.path,
            event.event_type,
        ],
    )?;
    Ok(())
}
