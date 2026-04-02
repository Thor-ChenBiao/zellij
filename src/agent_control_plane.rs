use humantime::format_rfc3339_seconds;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};
use uuid::Uuid;
use zellij_utils::consts::{VERSION, ZELLIJ_CACHE_DIR};

const ROOM_SCHEMA_VERSION: u32 = 1;
const REVIEW_TARGET_ROLE: &str = "driver";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentPersona {
    Claude,
    Codex,
    Gemini,
    Reviewer,
}

impl AgentPersona {
    pub(crate) fn provider_name(&self) -> &'static str {
        match self {
            AgentPersona::Claude => "claude",
            AgentPersona::Codex => "codex",
            AgentPersona::Gemini => "gemini",
            AgentPersona::Reviewer => "reviewer",
        }
    }

    pub(crate) fn role_name(&self) -> &'static str {
        match self {
            AgentPersona::Reviewer => "reviewer",
            _ => "driver",
        }
    }

    pub(crate) fn endpoint_label(&self) -> &'static str {
        match self {
            AgentPersona::Claude => "Claude Driver",
            AgentPersona::Codex => "Codex Driver",
            AgentPersona::Gemini => "Gemini Driver",
            AgentPersona::Reviewer => "Reviewer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewStreamKind {
    Events,
    Code,
}

impl ViewStreamKind {
    fn as_str(&self) -> &'static str {
        match self {
            ViewStreamKind::Events => "events",
            ViewStreamKind::Code => "code",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RoomMetadata {
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
pub(crate) struct EndpointMetadata {
    pub endpoint_id: String,
    pub shared_id: String,
    pub session_name: String,
    pub provider: String,
    pub role: String,
    pub label: String,
    pub cwd: String,
    pub pid: u32,
    pub created_at: String,
    pub reviewer_bootstrap_prompt: Option<String>,
    pub review_message_template: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RegisteredRoomParticipant {
    pub session_name: String,
    pub room_created: bool,
    pub endpoint: EndpointMetadata,
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

pub(crate) fn room_id_root() -> PathBuf {
    ZELLIJ_CACHE_DIR.join("agent-control-plane")
}

pub(crate) fn room_dir(shared_id: &str) -> PathBuf {
    room_id_root().join(shared_id)
}

pub(crate) fn room_metadata_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("room.json")
}

pub(crate) fn room_events_log_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("events.log")
}

pub(crate) fn room_events_jsonl_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("events.jsonl")
}

pub(crate) fn room_code_log_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("code.log")
}

pub(crate) fn room_code_jsonl_path(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("code.jsonl")
}

fn room_endpoints_dir(shared_id: &str) -> PathBuf {
    room_dir(shared_id).join("endpoints")
}

fn now_string() -> String {
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

fn reviewer_bootstrap_prompt(shared_id: &str, session_name: &str) -> String {
    format!(
        "You are the reviewer for shared room '{shared_id}' (session '{session_name}'). \
Review the driver's latest output, focus on correctness and risk, and send back concise, actionable feedback.",
    )
}

fn reviewer_message_template(label: &str) -> String {
    format!(
        "[Review from {label}]\nContext: shared-room review\nReview notes:\n- <summary>\n\nDetailed feedback:\n<feedback>"
    )
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

fn ensure_room_files(root: &Path, shared_id: &str, session_name: &str) -> io::Result<bool> {
    fs::create_dir_all(root.join(shared_id).join("endpoints"))?;
    let metadata_path = root.join(shared_id).join("room.json");
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
    fs::write(root.join(shared_id).join("events.log"), "")?;
    fs::write(root.join(shared_id).join("events.jsonl"), "")?;
    fs::write(root.join(shared_id).join("code.log"), "")?;
    fs::write(root.join(shared_id).join("code.jsonl"), "")?;
    Ok(true)
}

fn register_participant_at(
    root: &Path,
    shared_id: &str,
    persona: AgentPersona,
    cwd: &Path,
) -> io::Result<RegisteredRoomParticipant> {
    let session_name = shared_id.to_owned();
    let room_created = ensure_room_files(root, shared_id, &session_name)?;
    let endpoint_id = format!(
        "{}-{}",
        persona.provider_name(),
        &Uuid::new_v4().to_string()[..8]
    );
    let label = format!("{} {}", persona.endpoint_label(), &endpoint_id);
    let endpoint = EndpointMetadata {
        endpoint_id: endpoint_id.clone(),
        shared_id: shared_id.to_owned(),
        session_name: session_name.clone(),
        provider: persona.provider_name().to_owned(),
        role: persona.role_name().to_owned(),
        label: label.clone(),
        cwd: cwd.display().to_string(),
        pid: std::process::id(),
        created_at: now_string(),
        reviewer_bootstrap_prompt: (persona == AgentPersona::Reviewer)
            .then(|| reviewer_bootstrap_prompt(shared_id, &session_name)),
        review_message_template: (persona == AgentPersona::Reviewer)
            .then(|| reviewer_message_template(&label)),
    };
    write_json(
        &endpoint_metadata_path_in_root(root, shared_id, &endpoint_id),
        &endpoint,
    )?;
    append_stream_event_at(
        root,
        ViewStreamKind::Events,
        shared_id,
        "participant_registered",
        Some(&endpoint),
        format!(
            "{} joined room '{}' as {}",
            endpoint.provider, shared_id, endpoint.role
        ),
        json!({
            "room_created": room_created,
            "cwd": endpoint.cwd,
        }),
    )?;
    if persona == AgentPersona::Reviewer {
        append_stream_event_at(
            root,
            ViewStreamKind::Events,
            shared_id,
            "reviewer_bootstrap_ready",
            Some(&endpoint),
            "Reviewer bootstrap prompt and message template recorded".to_owned(),
            json!({
                "review_target_role": REVIEW_TARGET_ROLE,
                "message_template": endpoint.review_message_template,
            }),
        )?;
    }
    Ok(RegisteredRoomParticipant {
        session_name,
        room_created,
        endpoint,
    })
}

fn append_stream_event_at(
    root: &Path,
    stream: ViewStreamKind,
    shared_id: &str,
    kind: &str,
    endpoint: Option<&EndpointMetadata>,
    message: String,
    payload: serde_json::Value,
) -> io::Result<()> {
    let (text_path, json_path) = match stream {
        ViewStreamKind::Events => (
            root.join(shared_id).join("events.log"),
            root.join(shared_id).join("events.jsonl"),
        ),
        ViewStreamKind::Code => (
            root.join(shared_id).join("code.log"),
            root.join(shared_id).join("code.jsonl"),
        ),
    };
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

pub(crate) fn register_participant(
    shared_id: &str,
    persona: AgentPersona,
    cwd: &Path,
) -> io::Result<RegisteredRoomParticipant> {
    register_participant_at(&room_id_root(), shared_id, persona, cwd)
}

pub(crate) fn append_stream_event(
    stream: ViewStreamKind,
    shared_id: &str,
    kind: &str,
    endpoint: Option<&EndpointMetadata>,
    message: String,
    payload: serde_json::Value,
) -> io::Result<()> {
    append_stream_event_at(
        &room_id_root(),
        stream,
        shared_id,
        kind,
        endpoint,
        message,
        payload,
    )
}

pub(crate) fn room_metadata(shared_id: &str) -> io::Result<RoomMetadata> {
    let raw = fs::read_to_string(room_metadata_path(shared_id))?;
    serde_json::from_str(&raw).map_err(json_to_io_error)
}

pub(crate) fn list_room_endpoints(shared_id: &str) -> io::Result<Vec<EndpointMetadata>> {
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

pub(crate) fn ensure_room(shared_id: &str) -> io::Result<bool> {
    ensure_room_files(&room_id_root(), shared_id, shared_id)
}

fn endpoint_metadata_path_in_root(root: &Path, shared_id: &str, endpoint_id: &str) -> PathBuf {
    root.join(shared_id)
        .join("endpoints")
        .join(format!("{}.json", endpoint_id))
}

pub(crate) fn follow_stream(shared_id: &str, stream: ViewStreamKind) -> io::Result<()> {
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
    use tempfile::tempdir;

    #[test]
    fn registers_reviewer_with_prompt_and_template() {
        let temp = tempdir().unwrap();
        let shared_id = "team-42";
        let participant = register_participant_at(
            temp.path(),
            shared_id,
            AgentPersona::Reviewer,
            Path::new("/tmp/repo"),
        )
        .unwrap();

        assert!(participant.room_created);
        assert_eq!(participant.session_name, shared_id);
        assert_eq!(participant.endpoint.role, "reviewer");
        assert!(participant.endpoint.reviewer_bootstrap_prompt.is_some());
        assert!(participant.endpoint.review_message_template.is_some());
        assert!(temp.path().join(shared_id).join("events.log").exists());
        assert!(temp.path().join(shared_id).join("code.log").exists());
    }

    #[test]
    fn appends_structured_events_to_text_and_jsonl_streams() {
        let temp = tempdir().unwrap();
        let shared_id = "demo-room";
        let participant = register_participant_at(
            temp.path(),
            shared_id,
            AgentPersona::Claude,
            Path::new("/tmp/repo"),
        )
        .unwrap();

        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Code,
            shared_id,
            "file_changed",
            Some(&participant.endpoint),
            "Tracked a modified file".to_owned(),
            json!({ "file_path": "src/main.rs" }),
        )
        .unwrap();

        let raw_log = fs::read_to_string(temp.path().join(shared_id).join("code.log")).unwrap();
        let raw_jsonl = fs::read_to_string(temp.path().join(shared_id).join("code.jsonl")).unwrap();

        assert!(raw_log.contains("Tracked a modified file"));
        assert!(raw_jsonl.contains("\"file_changed\""));
        assert!(raw_jsonl.contains("\"src/main.rs\""));
    }
}
