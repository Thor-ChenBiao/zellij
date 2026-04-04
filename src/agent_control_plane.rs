use humantime::format_rfc3339_seconds;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};
use uuid::Uuid;
use zellij_utils::consts::{VERSION, ZELLIJ_CACHE_DIR};

const ROOM_SCHEMA_VERSION: u32 = 1;
const REVIEW_SCHEMA_VERSION: u32 = 1;
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
    pub bound_pane_id: Option<String>,
    pub bound_pane_id_updated_at: Option<String>,
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
pub(crate) struct ReviewRequest {
    pub schema_version: u32,
    pub room_id: String,
    pub request_id: String,
    pub driver_endpoint: Option<String>,
    pub target_pane_id: Option<String>,
    pub target_role: String,
    pub mode: String,
    pub reason: String,
    pub changed_files: Vec<String>,
    pub last_commands: Vec<String>,
    pub recent_output: Vec<String>,
    pub recent_code_snapshot: Vec<String>,
    pub task: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReviewFeedback {
    pub schema_version: u32,
    pub room_id: String,
    pub request_id: String,
    pub source_endpoint: String,
    pub target_role: String,
    pub severity: String,
    pub confidence: String,
    pub should_send: bool,
    pub summary: String,
    pub findings: Vec<String>,
    pub actions: Vec<String>,
    pub notes: Vec<String>,
    pub created_at: String,
    pub raw_text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PersistedReviewRequest {
    pub request: ReviewRequest,
    pub text: String,
    pub json_path: PathBuf,
    pub text_path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct PersistedReviewFeedback {
    pub feedback: ReviewFeedback,
    pub text: String,
    pub json_path: PathBuf,
    pub text_path: PathBuf,
    pub target_pane_id: Option<String>,
    pub driver_envelope: Option<String>,
    pub driver_envelope_path: Option<PathBuf>,
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

fn review_dir_in_root(root: &Path, shared_id: &str) -> PathBuf {
    root.join(shared_id).join("review")
}

fn review_requests_dir_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_dir_in_root(root, shared_id).join("requests")
}

fn review_feedback_dir_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_dir_in_root(root, shared_id).join("feedback")
}

fn review_inbox_dir_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_dir_in_root(root, shared_id).join("driver-inbox")
}

fn latest_review_request_json_path_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_requests_dir_in_root(root, shared_id).join("latest.json")
}

fn latest_review_request_text_path_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_requests_dir_in_root(root, shared_id).join("latest.txt")
}

fn review_request_history_jsonl_path_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_requests_dir_in_root(root, shared_id).join("history.jsonl")
}

fn latest_review_feedback_json_path_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_feedback_dir_in_root(root, shared_id).join("latest.json")
}

fn latest_review_feedback_text_path_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_feedback_dir_in_root(root, shared_id).join("latest.txt")
}

fn review_feedback_history_jsonl_path_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_feedback_dir_in_root(root, shared_id).join("history.jsonl")
}

fn latest_driver_envelope_path_in_root(root: &Path, shared_id: &str) -> PathBuf {
    review_inbox_dir_in_root(root, shared_id).join("latest.txt")
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
        bound_pane_id: None,
        bound_pane_id_updated_at: None,
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

fn room_metadata_at(root: &Path, shared_id: &str) -> io::Result<RoomMetadata> {
    let raw = fs::read_to_string(root.join(shared_id).join("room.json"))?;
    serde_json::from_str(&raw).map_err(json_to_io_error)
}

fn list_room_endpoints_at(root: &Path, shared_id: &str) -> io::Result<Vec<EndpointMetadata>> {
    let endpoints_dir = root.join(shared_id).join("endpoints");
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

fn update_endpoint_pane_binding_at(
    root: &Path,
    shared_id: &str,
    endpoint_id: &str,
    pane_id: &str,
) -> io::Result<()> {
    let endpoint_path = endpoint_metadata_path_in_root(root, shared_id, endpoint_id);
    if !endpoint_path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&endpoint_path)?;
    let mut endpoint: EndpointMetadata = serde_json::from_str(&raw).map_err(json_to_io_error)?;
    if endpoint.bound_pane_id.as_deref() == Some(pane_id) {
        return Ok(());
    }
    endpoint.bound_pane_id = Some(pane_id.to_owned());
    endpoint.bound_pane_id_updated_at = Some(now_string());
    write_json(&endpoint_path, &endpoint)
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    append_line(
        path,
        &serde_json::to_string(value).map_err(json_to_io_error)?,
    )
}

fn read_stream_events_at(
    root: &Path,
    shared_id: &str,
    stream: ViewStreamKind,
) -> io::Result<Vec<StreamEvent>> {
    let (_, jsonl_path) = match stream {
        ViewStreamKind::Events => (
            root.join(shared_id).join("events.log"),
            root.join(shared_id).join("events.jsonl"),
        ),
        ViewStreamKind::Code => (
            root.join(shared_id).join("code.log"),
            root.join(shared_id).join("code.jsonl"),
        ),
    };
    if !jsonl_path.exists() {
        return Ok(vec![]);
    }
    let raw = fs::read_to_string(jsonl_path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<StreamEvent>(line).map_err(json_to_io_error))
        .collect()
}

fn next_review_request_id(root: &Path, shared_id: &str) -> String {
    let count = fs::read_to_string(review_request_history_jsonl_path_in_root(root, shared_id))
        .ok()
        .map(|raw| raw.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0);
    format!("rr-{:04}", count + 1)
}

fn push_unique(vec: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    if seen.insert(value.clone()) {
        vec.push(value);
    }
}

fn synthesize_recent_code_snapshot(code_events: &[StreamEvent]) -> Vec<String> {
    code_events
        .iter()
        .rev()
        .find_map(|event| {
            event
                .payload
                .get("viewport")
                .and_then(|viewport| viewport.as_array())
                .map(|viewport| {
                    viewport
                        .iter()
                        .filter_map(|line| line.as_str().map(|line| line.to_owned()))
                        .take(12)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default()
}

fn event_target_pane_id(event: &StreamEvent) -> Option<String> {
    event
        .payload
        .get("pane_id")
        .and_then(|pane_id| pane_id.as_str())
        .map(|pane_id| pane_id.to_owned())
        .or_else(|| {
            event
                .payload
                .get("terminal_id")
                .and_then(|terminal_id| terminal_id.as_u64())
                .map(|terminal_id| format!("terminal_{}", terminal_id))
        })
}

fn is_driver_event(
    event: &StreamEvent,
    driver_endpoint: Option<&str>,
    driver_pane_id: Option<&str>,
) -> bool {
    if let Some(driver_endpoint) = driver_endpoint {
        if event.endpoint_id.as_deref() == Some(driver_endpoint) {
            return true;
        }
    }
    if let Some(driver_pane_id) = driver_pane_id {
        if event_target_pane_id(event).as_deref() == Some(driver_pane_id) {
            return true;
        }
    }
    if driver_endpoint.is_none() && driver_pane_id.is_none() {
        return event.role.as_deref() == Some(REVIEW_TARGET_ROLE);
    }
    false
}

fn infer_target_pane_id(
    events: &[StreamEvent],
    code_events: &[StreamEvent],
    driver_endpoint: Option<&str>,
    driver_pane_id: Option<&str>,
) -> Option<String> {
    if let Some(driver_pane_id) = driver_pane_id {
        return Some(driver_pane_id.to_owned());
    }
    code_events
        .iter()
        .rev()
        .filter(|event| is_driver_event(event, driver_endpoint, driver_pane_id))
        .find_map(event_target_pane_id)
        .or_else(|| {
            events
                .iter()
                .rev()
                .filter(|event| is_driver_event(event, driver_endpoint, driver_pane_id))
                .find_map(event_target_pane_id)
        })
        .or_else(|| {
            code_events
                .iter()
                .rev()
                .find_map(event_target_pane_id)
                .or_else(|| events.iter().rev().find_map(event_target_pane_id))
        })
}

fn synthesize_recent_code_snapshot_for_driver(
    code_events: &[StreamEvent],
    driver_endpoint: Option<&str>,
    driver_pane_id: Option<&str>,
) -> Vec<String> {
    code_events
        .iter()
        .rev()
        .filter(|event| is_driver_event(event, driver_endpoint, driver_pane_id))
        .find_map(|event| {
            event
                .payload
                .get("viewport")
                .and_then(|viewport| viewport.as_array())
                .map(|viewport| {
                    viewport
                        .iter()
                        .filter_map(|line| line.as_str().map(|line| line.to_owned()))
                        .take(12)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_else(|| synthesize_recent_code_snapshot(code_events))
}

fn render_list_section(title: &str, values: &[String]) -> String {
    let mut section = format!("{title}:\n");
    if values.is_empty() {
        section.push_str("- <none>\n");
    } else {
        for value in values {
            section.push_str(&format!("- {value}\n"));
        }
    }
    section
}

fn render_review_request_text(request: &ReviewRequest) -> String {
    let mut text = String::new();
    text.push_str("[ACP_REVIEW_REQUEST_BEGIN]\n");
    text.push_str(&format!("room_id: {}\n", request.room_id));
    text.push_str(&format!("request_id: {}\n", request.request_id));
    text.push_str(&format!(
        "driver_endpoint: {}\n",
        request
            .driver_endpoint
            .as_deref()
            .unwrap_or("<unknown-driver>")
    ));
    text.push_str(&format!(
        "target_pane_id: {}\n",
        request
            .target_pane_id
            .as_deref()
            .unwrap_or("<unknown-pane>")
    ));
    text.push_str(&format!("target_role: {}\n", request.target_role));
    text.push_str(&format!("mode: {}\n", request.mode));
    text.push_str(&format!("reason: {}\n\n", request.reason));
    text.push_str(&render_list_section(
        "changed_files",
        &request.changed_files,
    ));
    text.push('\n');
    text.push_str(&render_list_section(
        "last_commands",
        &request.last_commands,
    ));
    text.push('\n');
    text.push_str(&render_list_section(
        "recent_output",
        &request.recent_output,
    ));
    text.push('\n');
    text.push_str(&render_list_section(
        "recent_code_snapshot",
        &request.recent_code_snapshot,
    ));
    text.push('\n');
    text.push_str("task:\n");
    text.push_str(&request.task);
    text.push('\n');
    text.push_str("[ACP_REVIEW_REQUEST_END]\n");
    text
}

fn prepare_review_request_at(
    root: &Path,
    shared_id: &str,
    reason: Option<&str>,
) -> io::Result<PersistedReviewRequest> {
    let _ = room_metadata_at(root, shared_id)?;
    let endpoints = list_room_endpoints_at(root, shared_id)?;
    let driver_endpoint_metadata = endpoints
        .iter()
        .rev()
        .find(|endpoint| endpoint.role == REVIEW_TARGET_ROLE);
    let driver_endpoint = driver_endpoint_metadata.map(|endpoint| endpoint.endpoint_id.clone());
    let driver_pane_id =
        driver_endpoint_metadata.and_then(|endpoint| endpoint.bound_pane_id.clone());
    let events = read_stream_events_at(root, shared_id, ViewStreamKind::Events)?;
    let code_events = read_stream_events_at(root, shared_id, ViewStreamKind::Code)?;

    let mut changed_files = vec![];
    let mut changed_files_seen = HashSet::new();
    for event in &code_events {
        if !is_driver_event(event, driver_endpoint.as_deref(), driver_pane_id.as_deref()) {
            continue;
        }
        if let Some(file_path) = event.payload.get("file_path").and_then(|v| v.as_str()) {
            push_unique(
                &mut changed_files,
                &mut changed_files_seen,
                file_path.to_owned(),
            );
        }
    }

    let mut last_commands = vec![];
    let mut last_commands_seen = HashSet::new();
    for event in events.iter().rev() {
        if !is_driver_event(event, driver_endpoint.as_deref(), driver_pane_id.as_deref()) {
            continue;
        }
        if event.kind == "tool_call"
            && event
                .payload
                .get("tool")
                .and_then(|tool| tool.as_str())
                .map(|tool| tool == "Bash")
                .unwrap_or(false)
        {
            if let Some(command) = event.payload.get("value").and_then(|value| value.as_str()) {
                push_unique(
                    &mut last_commands,
                    &mut last_commands_seen,
                    command.to_owned(),
                );
            }
        }
        if last_commands.len() >= 5 {
            break;
        }
    }
    last_commands.reverse();

    let mut recent_output: Vec<String> = events
        .iter()
        .rev()
        .filter(|event| {
            is_driver_event(event, driver_endpoint.as_deref(), driver_pane_id.as_deref())
        })
        .filter(|event| {
            matches!(
                event.kind.as_str(),
                "tool_call"
                    | "write_character"
                    | "write_to_pane_id"
                    | "write_key_to_pane_id"
                    | "paste"
                    | "review_feedback_recorded"
            )
        })
        .map(|event| event.message.clone())
        .take(6)
        .collect();
    recent_output.reverse();

    let request = ReviewRequest {
        schema_version: REVIEW_SCHEMA_VERSION,
        room_id: shared_id.to_owned(),
        request_id: next_review_request_id(root, shared_id),
        driver_endpoint: driver_endpoint.clone(),
        target_pane_id: infer_target_pane_id(
            &events,
            &code_events,
            driver_endpoint.as_deref(),
            driver_pane_id.as_deref(),
        ),
        target_role: REVIEW_TARGET_ROLE.to_owned(),
        mode: "approval".to_owned(),
        reason: reason
            .unwrap_or("manual_request_from_room_stream")
            .to_owned(),
        changed_files,
        last_commands,
        recent_output,
        recent_code_snapshot: synthesize_recent_code_snapshot_for_driver(
            &code_events,
            driver_endpoint.as_deref(),
            driver_pane_id.as_deref(),
        ),
        task: "Review 最新的 driver 活动。只返回一个 ACP_REVIEW_RESPONSE_V1 block。".to_owned(),
        created_at: now_string(),
    };
    if let (Some(driver_endpoint), Some(target_pane_id)) = (
        request.driver_endpoint.as_deref(),
        request.target_pane_id.as_deref(),
    ) {
        update_endpoint_pane_binding_at(root, shared_id, driver_endpoint, target_pane_id)?;
    }
    let text = render_review_request_text(&request);
    let json_path = latest_review_request_json_path_in_root(root, shared_id);
    let text_path = latest_review_request_text_path_in_root(root, shared_id);
    write_json(&json_path, &request)?;
    fs::write(&text_path, &text)?;
    append_jsonl(
        &review_request_history_jsonl_path_in_root(root, shared_id),
        &request,
    )?;
    append_stream_event_at(
        root,
        ViewStreamKind::Events,
        shared_id,
        "review_request_generated",
        None,
        format!("Prepared review request {}", request.request_id),
        json!({
            "request_id": request.request_id,
            "driver_endpoint": request.driver_endpoint,
            "target_pane_id": request.target_pane_id,
            "changed_files": request.changed_files,
            "last_commands": request.last_commands,
        }),
    )?;
    Ok(PersistedReviewRequest {
        request,
        text,
        json_path,
        text_path,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedbackSection {
    Fields,
    Findings,
    Actions,
    Notes,
}

fn extract_review_feedback_block(raw: &str) -> io::Result<String> {
    let start_marker = "[ACP_REVIEW_RESPONSE_V1]";
    let end_marker = "[/ACP_REVIEW_RESPONSE_V1]";
    let start = raw.find(start_marker).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Missing ACP review response start marker",
        )
    })?;
    let end = raw.find(end_marker).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Missing ACP review response end marker",
        )
    })?;
    Ok(raw[start + start_marker.len()..end].trim().to_owned())
}

fn parse_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim()))
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "true" | "True" | "TRUE" | "yes" | "1")
}

fn parse_review_feedback(
    shared_id: &str,
    raw_text: &str,
    source_endpoint_override: Option<&str>,
) -> io::Result<ReviewFeedback> {
    let block = extract_review_feedback_block(raw_text)?;
    let mut room_id = None;
    let mut request_id = None;
    let mut source_endpoint = source_endpoint_override.map(|v| v.to_owned());
    let mut target_role = Some(REVIEW_TARGET_ROLE.to_owned());
    let mut severity = Some("medium".to_owned());
    let mut confidence = Some("medium".to_owned());
    let mut should_send = true;
    let mut summary = None;
    let mut findings = vec![];
    let mut actions = vec![];
    let mut notes = vec![];
    let mut section = FeedbackSection::Fields;

    for raw_line in block.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        match line {
            "findings:" => {
                section = FeedbackSection::Findings;
                continue;
            },
            "actions:" => {
                section = FeedbackSection::Actions;
                continue;
            },
            "notes:" => {
                section = FeedbackSection::Notes;
                continue;
            },
            _ => {},
        }
        if let Some(value) = line.strip_prefix("- ") {
            match section {
                FeedbackSection::Findings => findings.push(value.to_owned()),
                FeedbackSection::Actions => actions.push(value.to_owned()),
                FeedbackSection::Notes => notes.push(value.to_owned()),
                FeedbackSection::Fields => {},
            }
            continue;
        }
        if let Some((key, value)) = parse_key_value(line) {
            match key {
                "room_id" => room_id = Some(value.to_owned()),
                "request_id" => request_id = Some(value.to_owned()),
                "source_endpoint" => source_endpoint = Some(value.to_owned()),
                "target_role" => target_role = Some(value.to_owned()),
                "severity" => severity = Some(value.to_owned()),
                "confidence" => confidence = Some(value.to_owned()),
                "should_send" => should_send = parse_bool(value),
                "summary" => summary = Some(value.to_owned()),
                _ => {},
            }
        }
    }

    let room_id = room_id.unwrap_or_else(|| shared_id.to_owned());
    if room_id != shared_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "Feedback room_id '{}' does not match target room '{}'",
                room_id, shared_id
            ),
        ));
    }
    let request_id = request_id.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Feedback is missing request_id",
        )
    })?;
    let source_endpoint = source_endpoint.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Feedback is missing source_endpoint",
        )
    })?;
    let summary = summary.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Feedback is missing summary")
    })?;

    Ok(ReviewFeedback {
        schema_version: REVIEW_SCHEMA_VERSION,
        room_id,
        request_id,
        source_endpoint,
        target_role: target_role.unwrap_or_else(|| REVIEW_TARGET_ROLE.to_owned()),
        severity: severity.unwrap_or_else(|| "medium".to_owned()),
        confidence: confidence.unwrap_or_else(|| "medium".to_owned()),
        should_send,
        summary,
        findings,
        actions,
        notes,
        created_at: now_string(),
        raw_text: raw_text.to_owned(),
    })
}

fn render_review_feedback_text(feedback: &ReviewFeedback) -> String {
    let mut text = String::new();
    text.push_str("[ACP_REVIEW_RESPONSE_V1]\n");
    text.push_str(&format!("room_id: {}\n", feedback.room_id));
    text.push_str(&format!("request_id: {}\n", feedback.request_id));
    text.push_str(&format!("source_endpoint: {}\n", feedback.source_endpoint));
    text.push_str(&format!("target_role: {}\n", feedback.target_role));
    text.push_str(&format!("severity: {}\n", feedback.severity));
    text.push_str(&format!("confidence: {}\n", feedback.confidence));
    text.push_str(&format!("should_send: {}\n", feedback.should_send));
    text.push_str(&format!("summary: {}\n\n", feedback.summary));
    text.push_str(&render_list_section("findings", &feedback.findings));
    text.push('\n');
    text.push_str(&render_list_section("actions", &feedback.actions));
    text.push('\n');
    text.push_str(&render_list_section("notes", &feedback.notes));
    text.push_str("[/ACP_REVIEW_RESPONSE_V1]\n");
    text
}

fn render_driver_feedback_envelope(feedback: &ReviewFeedback) -> String {
    let mut text = String::new();
    text.push_str("[ACP_MESSAGE_BEGIN]\n");
    text.push_str(&format!("room_id: {}\n", feedback.room_id));
    text.push_str("message_type: review_feedback\n");
    text.push_str("source_role: reviewer\n");
    text.push_str(&format!("source_endpoint: {}\n", feedback.source_endpoint));
    text.push_str(&format!("target_role: {}\n", feedback.target_role));
    text.push_str(&format!("request_id: {}\n", feedback.request_id));
    text.push_str("delivery_mode: approval\n");
    text.push_str(&format!("severity: {}\n", feedback.severity));
    text.push_str(&format!("summary: {}\n", feedback.summary));
    text.push_str("[ACP_MESSAGE_END]\n\n");
    text.push_str(&format!("[Review from {}]\n", feedback.source_endpoint));
    if !feedback.findings.is_empty() {
        for finding in &feedback.findings {
            text.push_str(&format!("- {}\n", finding));
        }
    } else {
        text.push_str(&format!("- {}\n", feedback.summary));
    }
    if !feedback.actions.is_empty() {
        text.push('\n');
        text.push_str("Suggested actions:\n");
        for action in &feedback.actions {
            text.push_str(&format!("- {}\n", action));
        }
    }
    if !feedback.notes.is_empty() {
        text.push('\n');
        text.push_str("Notes:\n");
        for note in &feedback.notes {
            text.push_str(&format!("- {}\n", note));
        }
    }
    text
}

fn find_review_request_by_id(
    root: &Path,
    shared_id: &str,
    request_id: &str,
) -> io::Result<Option<ReviewRequest>> {
    let latest_path = latest_review_request_json_path_in_root(root, shared_id);
    if latest_path.exists() {
        let raw = fs::read_to_string(&latest_path)?;
        let latest: ReviewRequest = serde_json::from_str(&raw).map_err(json_to_io_error)?;
        if latest.request_id == request_id {
            return Ok(Some(latest));
        }
    }
    let history_path = review_request_history_jsonl_path_in_root(root, shared_id);
    if !history_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(history_path)?;
    for line in raw.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let request: ReviewRequest = serde_json::from_str(line).map_err(json_to_io_error)?;
        if request.request_id == request_id {
            return Ok(Some(request));
        }
    }
    Ok(None)
}

fn resolve_feedback_target_pane_id(
    root: &Path,
    shared_id: &str,
    request_id: &str,
) -> io::Result<Option<String>> {
    if let Some(request) = find_review_request_by_id(root, shared_id, request_id)? {
        if request.target_pane_id.is_some() {
            return Ok(request.target_pane_id);
        }
        if let Some(driver_endpoint) = request.driver_endpoint.as_deref() {
            let endpoints = list_room_endpoints_at(root, shared_id)?;
            if let Some(bound_pane_id) = endpoints
                .iter()
                .find(|endpoint| endpoint.endpoint_id == driver_endpoint)
                .and_then(|endpoint| endpoint.bound_pane_id.clone())
            {
                return Ok(Some(bound_pane_id));
            }
        }
    }
    let events = read_stream_events_at(root, shared_id, ViewStreamKind::Events)?;
    let code_events = read_stream_events_at(root, shared_id, ViewStreamKind::Code)?;
    Ok(infer_target_pane_id(&events, &code_events, None, None))
}

fn record_review_feedback_at(
    root: &Path,
    shared_id: &str,
    raw_feedback: &str,
    source_endpoint_override: Option<&str>,
) -> io::Result<PersistedReviewFeedback> {
    let _ = room_metadata_at(root, shared_id)?;
    let feedback = parse_review_feedback(shared_id, raw_feedback, source_endpoint_override)?;
    let text = render_review_feedback_text(&feedback);
    let json_path = latest_review_feedback_json_path_in_root(root, shared_id);
    let text_path = latest_review_feedback_text_path_in_root(root, shared_id);
    write_json(&json_path, &feedback)?;
    fs::write(&text_path, &text)?;
    append_jsonl(
        &review_feedback_history_jsonl_path_in_root(root, shared_id),
        &feedback,
    )?;
    append_stream_event_at(
        root,
        ViewStreamKind::Events,
        shared_id,
        "review_feedback_recorded",
        None,
        format!(
            "Recorded review feedback {} from {}",
            feedback.request_id, feedback.source_endpoint
        ),
        json!({
            "request_id": feedback.request_id,
            "source_endpoint": feedback.source_endpoint,
            "severity": feedback.severity,
            "should_send": feedback.should_send,
        }),
    )?;
    let target_pane_id = if feedback.should_send {
        resolve_feedback_target_pane_id(root, shared_id, &feedback.request_id)?
    } else {
        None
    };
    let (driver_envelope, driver_envelope_path) = if feedback.should_send {
        let envelope = render_driver_feedback_envelope(&feedback);
        let envelope_path = latest_driver_envelope_path_in_root(root, shared_id);
        if let Some(parent) = envelope_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&envelope_path, &envelope)?;
        append_stream_event_at(
            root,
            ViewStreamKind::Events,
            shared_id,
            "review_feedback_ready_for_driver",
            None,
            format!("Prepared driver envelope for {}", feedback.request_id),
            json!({
                "request_id": feedback.request_id,
                "target_role": feedback.target_role,
                "target_pane_id": target_pane_id,
                "envelope_path": envelope_path.display().to_string(),
            }),
        )?;
        (Some(envelope), Some(envelope_path))
    } else {
        (None, None)
    };
    Ok(PersistedReviewFeedback {
        feedback,
        text,
        json_path,
        text_path,
        target_pane_id,
        driver_envelope,
        driver_envelope_path,
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

pub(crate) fn update_endpoint_pane_binding(
    shared_id: &str,
    endpoint_id: &str,
    pane_id: &str,
) -> io::Result<()> {
    update_endpoint_pane_binding_at(&room_id_root(), shared_id, endpoint_id, pane_id)
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

pub(crate) fn prepare_review_request(shared_id: &str) -> io::Result<PersistedReviewRequest> {
    prepare_review_request_at(
        &room_id_root(),
        shared_id,
        Some("manual_request_from_room_stream"),
    )
}

pub(crate) fn record_review_feedback(
    shared_id: &str,
    raw_feedback: &str,
    source_endpoint_override: Option<&str>,
) -> io::Result<PersistedReviewFeedback> {
    record_review_feedback_at(
        &room_id_root(),
        shared_id,
        raw_feedback,
        source_endpoint_override,
    )
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

    #[test]
    fn prepares_review_request_from_room_streams() {
        let temp = tempdir().unwrap();
        let shared_id = "review-room";
        let participant = register_participant_at(
            temp.path(),
            shared_id,
            AgentPersona::Claude,
            Path::new("/tmp/repo"),
        )
        .unwrap();

        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Events,
            shared_id,
            "tool_call",
            Some(&participant.endpoint),
            "Bash(cargo test -p zellij-server)".to_owned(),
            json!({
                "tool": "Bash",
                "value": "cargo test -p zellij-server",
            }),
        )
        .unwrap();
        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Code,
            shared_id,
            "file_hint",
            Some(&participant.endpoint),
            "Detected file hint src/main.rs".to_owned(),
            json!({
                "file_path": "src/main.rs",
            }),
        )
        .unwrap();
        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Code,
            shared_id,
            "pane_snapshot",
            Some(&participant.endpoint),
            "Pane terminal_1 viewport updated".to_owned(),
            json!({
                "pane_id": "terminal_1",
                "viewport": ["fn main() {", "    println!(\"hi\");", "}"],
            }),
        )
        .unwrap();

        let persisted =
            prepare_review_request_at(temp.path(), shared_id, Some("driver_turn_closed")).unwrap();
        let endpoints = list_room_endpoints_at(temp.path(), shared_id).unwrap();
        let driver_endpoint = endpoints
            .iter()
            .find(|endpoint| endpoint.role == REVIEW_TARGET_ROLE)
            .unwrap();

        assert_eq!(persisted.request.reason, "driver_turn_closed");
        assert_eq!(persisted.request.changed_files, vec!["src/main.rs"]);
        assert_eq!(
            persisted.request.target_pane_id.as_deref(),
            Some("terminal_1")
        );
        assert_eq!(driver_endpoint.bound_pane_id.as_deref(), Some("terminal_1"));
        assert_eq!(
            persisted.request.last_commands,
            vec!["cargo test -p zellij-server"]
        );
        assert!(persisted
            .request
            .recent_code_snapshot
            .contains(&"fn main() {".to_owned()));
        assert!(persisted.text.contains("[ACP_REVIEW_REQUEST_BEGIN]"));
        assert!(persisted.json_path.exists());
        assert!(persisted.text_path.exists());
    }

    #[test]
    fn prepares_review_request_from_driver_events_only_when_reviewer_is_present() {
        let temp = tempdir().unwrap();
        let shared_id = "review-room-mixed";
        let driver = register_participant_at(
            temp.path(),
            shared_id,
            AgentPersona::Claude,
            Path::new("/tmp/repo"),
        )
        .unwrap();
        let reviewer = register_participant_at(
            temp.path(),
            shared_id,
            AgentPersona::Reviewer,
            Path::new("/tmp/repo"),
        )
        .unwrap();

        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Events,
            shared_id,
            "tool_call",
            Some(&driver.endpoint),
            "Bash(cargo test -p zellij-server)".to_owned(),
            json!({
                "tool": "Bash",
                "value": "cargo test -p zellij-server",
                "pane_id": "terminal_1",
            }),
        )
        .unwrap();
        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Code,
            shared_id,
            "file_hint",
            Some(&driver.endpoint),
            "Detected file hint src/main.rs".to_owned(),
            json!({
                "file_path": "src/main.rs",
                "pane_id": "terminal_1",
            }),
        )
        .unwrap();
        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Code,
            shared_id,
            "pane_snapshot",
            Some(&driver.endpoint),
            "Pane terminal_1 viewport updated".to_owned(),
            json!({
                "pane_id": "terminal_1",
                "viewport": ["driver line 1", "driver line 2"],
            }),
        )
        .unwrap();
        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Events,
            shared_id,
            "paste",
            Some(&reviewer.endpoint),
            "Input reviewer bootstrap -> terminal_9".to_owned(),
            json!({
                "pane_id": "terminal_9",
            }),
        )
        .unwrap();
        append_stream_event_at(
            temp.path(),
            ViewStreamKind::Code,
            shared_id,
            "pane_snapshot",
            Some(&reviewer.endpoint),
            "Pane terminal_9 viewport updated".to_owned(),
            json!({
                "pane_id": "terminal_9",
                "viewport": ["reviewer line 1", "reviewer line 2"],
            }),
        )
        .unwrap();

        let persisted =
            prepare_review_request_at(temp.path(), shared_id, Some("driver_turn_closed")).unwrap();

        assert_eq!(
            persisted.request.target_pane_id.as_deref(),
            Some("terminal_1")
        );
        assert_eq!(persisted.request.changed_files, vec!["src/main.rs"]);
        assert_eq!(
            persisted.request.last_commands,
            vec!["cargo test -p zellij-server"]
        );
        assert_eq!(
            persisted.request.recent_code_snapshot,
            vec!["driver line 1".to_owned(), "driver line 2".to_owned()]
        );
        assert!(persisted
            .request
            .recent_output
            .iter()
            .all(|line| !line.contains("reviewer bootstrap")));
    }

    #[test]
    fn records_review_feedback_and_prepares_driver_envelope() {
        let temp = tempdir().unwrap();
        let shared_id = "feedback-room";
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
            "pane_snapshot",
            Some(&participant.endpoint),
            "Pane terminal_1 viewport updated".to_owned(),
            json!({
                "pane_id": "terminal_1",
                "viewport": ["fn main() {", "}"],
            }),
        )
        .unwrap();
        prepare_review_request_at(temp.path(), shared_id, Some("driver_turn_closed")).unwrap();

        let raw_feedback = r#"
[ACP_REVIEW_RESPONSE_V1]
room_id: feedback-room
request_id: rr-0001
source_endpoint: reviewer-7c31
target_role: driver
severity: medium
confidence: high
should_send: true
summary: 缺少回归测试。

findings:
- 当前没有 focused regression test。

actions:
- 补一条 focused regression test。

notes:
- 改动面比较小。
[/ACP_REVIEW_RESPONSE_V1]
"#;

        let persisted =
            record_review_feedback_at(temp.path(), shared_id, raw_feedback, None).unwrap();

        assert_eq!(persisted.feedback.request_id, "rr-0001");
        assert_eq!(persisted.target_pane_id.as_deref(), Some("terminal_1"));
        assert!(persisted.text.contains("[ACP_REVIEW_RESPONSE_V1]"));
        assert!(persisted.driver_envelope.is_some());
        assert!(persisted
            .driver_envelope
            .as_deref()
            .unwrap()
            .contains("[ACP_MESSAGE_BEGIN]"));
        assert!(persisted.driver_envelope_path.unwrap().exists());
    }
}
