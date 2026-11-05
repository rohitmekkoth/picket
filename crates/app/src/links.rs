//! Opens a fixed set of pages in the default browser or System Settings. The webview
//! names a destination and, for Gmail, a thread id; it never supplies a URL.

use std::process::Command;

use tauri::Manager;

use crate::GoogleState;

pub const GOOGLE_ACCESS: &str = "https://myaccount.google.com/connections";
const NOTIFICATION_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.Notifications-Settings.extension";
const FULL_DISK_ACCESS: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

#[tauri::command]
pub fn open_link(
    app: tauri::AppHandle,
    target: String,
    thread: Option<String>,
) -> Result<(), String> {
    let url = match target.as_str() {
        "gmail_thread" => {
            let thread = thread.ok_or("missing thread")?;
            if thread.is_empty()
                || thread.len() > 64
                || !thread.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return Err("invalid thread".into());
            }
            // Gmail picks the right signed-in account by its address.
            let account = app
                .state::<GoogleState>()
                .session
                .lock()
                .ok()
                .and_then(|session| {
                    session
                        .as_ref()
                        .map(|session| session.credentials.account_email.clone())
                });
            match account {
                Some(email) => format!(
                    "https://mail.google.com/mail/?authuser={}#all/{thread}",
                    encode(&email)
                ),
                None => format!("https://mail.google.com/mail/#all/{thread}"),
            }
        }
        "google_access" => GOOGLE_ACCESS.to_owned(),
        "notification_settings" => NOTIFICATION_SETTINGS.to_owned(),
        "full_disk_access" => FULL_DISK_ACCESS.to_owned(),
        _ => return Err("unsupported link".into()),
    };
    Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|_| "the page could not be opened".to_string())
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| "the page could not be opened".to_string())
        })
}

/// Percent-encodes everything except unreserved characters.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
