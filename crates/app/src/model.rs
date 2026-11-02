//! The local extraction model: downloaded only when the user asks, from Hugging Face
//! hosts only, resumable, and installed only if it matches its pinned checksum.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
    thread,
    time::Duration,
};

use picket_model::{ModelSpec, verify};
use reqwest::{Url, blocking::Client, redirect::Policy};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::Manager;

/// The default extractor, chosen by the evaluation recorded in docs/decisions.md.
pub const DEFAULT: &str = "qwen3.5-4b-q4_k_m";

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelStatus {
    Absent,
    Downloading { received: u64, total: u64 },
    Ready,
    Failed { reason: &'static str },
}

pub struct ModelState(pub Mutex<ModelStatus>);

impl Default for ModelState {
    fn default() -> Self {
        Self(Mutex::new(ModelStatus::Absent))
    }
}

pub fn spec() -> Option<ModelSpec> {
    picket_model::spec(DEFAULT)
}

/// Where the verified model file lives; it is outside the event log and never exported.
pub fn path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let directory = app.path().app_data_dir().ok()?.join("models");
    Some(directory.join(spec()?.file))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    status: ModelStatus,
    id: &'static str,
    bytes: u64,
}

#[tauri::command]
pub fn model_status(app: tauri::AppHandle) -> Result<ModelView, String> {
    let spec = spec().ok_or("model unavailable")?;
    let mut status = app
        .state::<ModelState>()
        .0
        .lock()
        .map_err(|_| "model state unavailable")?
        .clone();
    if matches!(status, ModelStatus::Absent) && path(&app).is_some_and(|path| path.exists()) {
        status = ModelStatus::Ready;
    }
    Ok(ModelView {
        status,
        id: spec.id,
        bytes: spec.bytes,
    })
}

/// Starts the download. Picket makes no request to Hugging Face until this is called.
#[tauri::command]
pub fn download_model(app: tauri::AppHandle) -> Result<(), String> {
    {
        let state = app.state::<ModelState>();
        let mut status = state.0.lock().map_err(|_| "model state unavailable")?;
        if matches!(*status, ModelStatus::Downloading { .. }) {
            return Ok(());
        }
        *status = ModelStatus::Downloading {
            received: 0,
            total: 0,
        };
    }
    thread::Builder::new()
        .name("picket-model-download".into())
        .spawn(move || {
            let result = download(&app);
            set(
                &app,
                match result {
                    Ok(()) => ModelStatus::Ready,
                    Err(reason) => ModelStatus::Failed { reason },
                },
            );
        })
        .map(|_| ())
        .map_err(|_| "download could not start".into())
}

fn set(app: &tauri::AppHandle, value: ModelStatus) {
    if let Ok(mut status) = app.state::<ModelState>().0.lock() {
        *status = value;
    }
}

/// Hugging Face serves files from huggingface.co and redirects to its own CDN hosts.
fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host == "huggingface.co" || host.ends_with(".hf.co"))
}

fn download(app: &tauri::AppHandle) -> Result<(), &'static str> {
    let spec = spec().ok_or("model unavailable")?;
    let target = path(app).ok_or("data path unavailable")?;
    let directory = target.parent().ok_or("data path unavailable")?;
    fs::create_dir_all(directory).map_err(|_| "model folder unavailable")?;
    let partial = target.with_extension("part");
    let client = Client::builder()
        .https_only(true)
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "network client unavailable")?;
    // Resume from a partial file, hashing what is already there.
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    if let Ok(mut existing) = File::open(&partial) {
        let mut buffer = vec![0; 1 << 20];
        loop {
            let read = existing
                .read(&mut buffer)
                .map_err(|_| "partial file unreadable")?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            received += read as u64;
        }
    }
    let mut request = client.get(spec.url());
    if received > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={received}-"));
    }
    let mut response = request.send().map_err(|_| "download failed")?;
    if !allowed(response.url()) {
        return Err("download redirected outside Hugging Face");
    }
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !resumed {
        if !response.status().is_success() {
            return Err("download refused");
        }
        hasher = Sha256::new();
        received = 0;
    }
    let total = received + response.content_length().unwrap_or(0);
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&partial)
        .map_err(|_| "model file unwritable")?;
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|_| "download interrupted")?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|_| "model file unwritable")?;
        hasher.update(&buffer[..read]);
        received += read as u64;
        set(app, ModelStatus::Downloading { received, total });
    }
    file.sync_all().map_err(|_| "model file unwritable")?;
    if format!("{:x}", hasher.finalize()) != spec.sha256 {
        let _ = fs::remove_file(&partial);
        return Err("downloaded file failed its checksum");
    }
    fs::rename(&partial, &target).map_err(|_| "model file unwritable")?;
    verify(&target, &spec).map_err(|_| "downloaded file failed its checksum")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hugging_face_https_hosts_are_followed() -> Result<(), Box<dyn std::error::Error>> {
        for (url, expected) in [
            ("https://huggingface.co/x", true),
            ("https://us.aws.cdn.hf.co/x", true),
            ("http://huggingface.co/x", false),
            ("https://huggingface.co.example.test/x", false),
            ("https://hf.co.example.test/x", false),
            ("https://example.test/x", false),
        ] {
            assert_eq!(allowed(&Url::parse(url)?), expected, "{url}");
        }
        Ok(())
    }
}
