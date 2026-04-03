use humantime::format_rfc3339_seconds;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

use crate::consts::{VERSION, ZELLIJ_CACHE_DIR};

const ROOM_SCHEMA_VERSION: u32 = 1;
const ROOM_HINT_DIR: &str = "/tmp/zellij-agent-control-plane";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewStreamKind {
    Events,
    Code,
}

impl ViewStreamKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ViewStreamKind::Events => "events",
            ViewStreamKind::Code => "code",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMetadata {
    pub schema_version: u32,
    pub shared_id: String,
    pub session_name: String,
    pub created_at: String,
    pub updated_at: String,
    pub zellij_version: String,
    pub bridge_transport: String,
    pub network_transport: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointMetadata {
    pub endpoint_id: String,
    pub shared_id: String,
    pub session_name: String,
    pub provider: String,
    pub role: String,
    pub label: String,
    pub cwd: String,
    pub pid: u32,
    pub created_at: String,
    pub bound_pane_id: Option<String>,
    pub bound_pane_id_updated_at: Option<String>,
    pub reviewer_bootstrap_prompt: Option<String>,
    pub review_message_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StreamEvent {
    time: String,
    stream: String,
    kind: String,
    shared_id: String,
    endpoint_id: Option<String>,
    provider: Option<String>,
    role: Option<String>,
    message: String,
    payload: serde_json::Value,
}

pub fn room_id_root() -> PathBuf {
    ZELLIJ_CACHE_DIR.join("agent-control-plane")
}

pub fn room_dir(shared_id: &str) -> PathBuf {
    room_id_root().join(shared_id)
}

pub fn room_hint_path(shared_id: &str) -> PathBuf {
    PathBuf::from(ROOM_HINT_DIR).join(format!("{}.room", shared_id))
}

pub fn room_metadata_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("room.json")
}

pub fn room_events_log_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("events.log")
}

pub fn room_events_jsonl_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("events.jsonl")
}

pub fn room_code_log_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("code.log")
}

pub fn room_code_jsonl_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("code.jsonl")
}

fn room_endpoints_dir(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("endpoints")
}

pub fn endpoint_metadata_path(shared_id: &str, endpoint_id: &str) -> PathBuf {
    room_endpoints_dir(shared_id).join(format!("{}.json", endpoint_id))
}

pub fn now_string() -> String {
    format_rfc3339_seconds(SystemTime::now()).to_string()
}

fn json_to_io_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn append_line(path: &Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", line)?;
    file.flush()?;
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_vec_pretty(value).map_err(json_to_io_error)?;
    fs::write(path, raw)?;
    Ok(())
}

fn stream_paths(shared_id: &str, stream: ViewStreamKind) -> (PathBuf, PathBuf) {
    match stream {
        ViewStreamKind::Events => (
            room_events_log_path(shared_id),
            room_events_jsonl_path(shared_id),
        ),
        ViewStreamKind::Code => (
            room_code_log_path(shared_id),
            room_code_jsonl_path(shared_id),
        ),
    }
}

pub fn room_exists(shared_id: &str) -> bool {
    room_metadata_path(shared_id).exists()
}

pub fn ensure_room_files(shared_id: &str, session_name: &str) -> io::Result<bool> {
    fs::create_dir_all(room_endpoints_dir(shared_id))?;
    fs::create_dir_all(ROOM_HINT_DIR)?;
    let metadata_path = room_metadata_path(shared_id);
    fs::write(room_hint_path(shared_id), format!("{}\n", session_name))?;
    if metadata_path.exists() {
        let raw = fs::read_to_string(&metadata_path)?;
        let mut metadata: RoomMetadata = serde_json::from_str(&raw).map_err(json_to_io_error)?;
        metadata.updated_at = now_string();
        write_json(&metadata_path, &metadata)?;
        return Ok(false);
    }

    let now = now_string();
    let metadata = RoomMetadata {
        schema_version: ROOM_SCHEMA_VERSION,
        shared_id: shared_id.to_owned(),
        session_name: session_name.to_owned(),
        created_at: now.clone(),
        updated_at: now,
        zellij_version: VERSION.to_owned(),
        bridge_transport: "local-room-cache".to_owned(),
        network_transport: "websocket-planned".to_owned(),
    };
    write_json(&metadata_path, &metadata)?;
    fs::write(room_events_log_path(shared_id), "")?;
    fs::write(room_events_jsonl_path(shared_id), "")?;
    fs::write(room_code_log_path(shared_id), "")?;
    fs::write(room_code_jsonl_path(shared_id), "")?;
    Ok(true)
}

pub fn ensure_room(shared_id: &str) -> io::Result<bool> {
    ensure_room_files(shared_id, shared_id)
}

pub fn write_endpoint_metadata(shared_id: &str, endpoint: &EndpointMetadata) -> io::Result<()> {
    write_json(
        &endpoint_metadata_path(shared_id, &endpoint.endpoint_id),
        endpoint,
    )
}

pub fn append_stream_event(
    stream: ViewStreamKind,
    shared_id: &str,
    kind: &str,
    endpoint: Option<&EndpointMetadata>,
    message: String,
    payload: serde_json::Value,
) -> io::Result<()> {
    let (text_path, json_path) = stream_paths(shared_id, stream);
    let time = now_string();
    let event = StreamEvent {
        time: time.clone(),
        stream: stream.as_str().to_owned(),
        kind: kind.to_owned(),
        shared_id: shared_id.to_owned(),
        endpoint_id: endpoint.map(|e| e.endpoint_id.clone()),
        provider: endpoint.map(|e| e.provider.clone()),
        role: endpoint.map(|e| e.role.clone()),
        message: message.clone(),
        payload,
    };
    let text_line = if let Some(endpoint) = endpoint {
        format!(
            "{} | {} | {} | {} | {}",
            time, endpoint.provider, endpoint.role, kind, message
        )
    } else {
        format!("{} | room | {} | {}", time, kind, message)
    };
    append_line(&text_path, &text_line)?;
    append_line(
        &json_path,
        &serde_json::to_string(&event).map_err(json_to_io_error)?,
    )?;
    Ok(())
}

pub fn append_session_stream_event(
    stream: ViewStreamKind,
    session_name: &str,
    kind: &str,
    message: String,
    payload: serde_json::Value,
) -> io::Result<bool> {
    if !room_exists(session_name) {
        return Ok(false);
    }
    append_stream_event(stream, session_name, kind, None, message, payload)?;
    Ok(true)
}

pub fn room_metadata(shared_id: &str) -> io::Result<RoomMetadata> {
    let raw = fs::read_to_string(room_metadata_path(shared_id))?;
    serde_json::from_str(&raw).map_err(json_to_io_error)
}

pub fn list_room_endpoints(shared_id: &str) -> io::Result<Vec<EndpointMetadata>> {
    let endpoints_dir = room_endpoints_dir(shared_id);
    if !endpoints_dir.exists() {
        return Ok(vec![]);
    }
    let mut endpoints = vec![];
    for entry in fs::read_dir(endpoints_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let raw = fs::read_to_string(entry.path())?;
            let endpoint: EndpointMetadata =
                serde_json::from_str(&raw).map_err(json_to_io_error)?;
            endpoints.push(endpoint);
        }
    }
    endpoints.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(endpoints)
}

pub fn follow_stream(shared_id: &str, stream: ViewStreamKind) -> io::Result<()> {
    ensure_room(shared_id)?;
    let (text_path, _) = stream_paths(shared_id, stream);
    let mut offset = 0usize;
    loop {
        let content = fs::read_to_string(&text_path).unwrap_or_default();
        if content.len() < offset {
            offset = 0;
        }
        if content.len() > offset {
            print!("{}", &content[offset..]);
            std::io::stdout().flush()?;
            offset = content.len();
        }
        thread::sleep(Duration::from_millis(350));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn ensure_room_files_creates_expected_streams() {
        let test_shared_id = format!("room-{}", &Uuid::new_v4().to_string()[..8]);
        let _ = fs::remove_dir_all(room_dir(&test_shared_id));

        let created = ensure_room_files(&test_shared_id, &test_shared_id).unwrap();

        assert!(created);
        assert!(room_metadata_path(&test_shared_id).exists());
        assert!(room_events_log_path(&test_shared_id).exists());
        assert!(room_code_jsonl_path(&test_shared_id).exists());

        let _ = fs::remove_dir_all(room_dir(&test_shared_id));
    }

    #[test]
    fn appends_event_into_both_stream_files() {
        let test_shared_id = format!("room-{}", &Uuid::new_v4().to_string()[..8]);
        let _ = fs::remove_dir_all(room_dir(&test_shared_id));

        ensure_room_files(&test_shared_id, &test_shared_id).unwrap();
        append_stream_event(
            ViewStreamKind::Events,
            &test_shared_id,
            "tool_call",
            None,
            "Detected Bash(ls -la)".to_owned(),
            json!({"tool": "Bash", "value": "ls -la"}),
        )
        .unwrap();

        let log = fs::read_to_string(room_events_log_path(&test_shared_id)).unwrap();
        let jsonl = fs::read_to_string(room_events_jsonl_path(&test_shared_id)).unwrap();

        assert!(log.contains("Detected Bash(ls -la)"));
        assert!(jsonl.contains("\"tool_call\""));
        assert!(jsonl.contains("\"ls -la\""));

        let _ = fs::remove_dir_all(room_dir(&test_shared_id));
    }
}
