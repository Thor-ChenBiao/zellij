use crate::agent_control_plane::{
    append_stream_event, ensure_provider_hooks, follow_stream, list_room_endpoints,
    prepare_review_request, record_review_feedback, register_participant, room_id_root,
    room_metadata, set_room_driver_provider_args, set_room_reviewer_prompt_override,
    set_room_reviewer_target_count, update_endpoint_pane_binding, AgentPersona, EndpointMetadata,
    PersistedReviewFeedback, PersistedReviewRequest, ViewStreamKind,
};
use dialoguer::Confirm;
use humantime::{format_rfc3339_seconds, parse_rfc3339};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::{
    collections::HashSet,
    fs::File,
    io,
    io::prelude::*,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, SystemTime},
};
use std::{fs, process::Stdio};

#[cfg(feature = "web_server_capability")]
use isahc::{config::RedirectPolicy, prelude::*, HttpClient, Request};

use zellij_client::{
    old_config_converter::{
        config_yaml_to_config_kdl, convert_old_yaml_files, layout_yaml_to_layout_kdl,
    },
    os_input_output::get_client_os_input,
    start_client as start_client_impl, ClientInfo,
};

use zellij_utils::sessions::{
    assert_dead_session, assert_session, assert_session_ne, delete_session as delete_session_impl,
    generate_unique_session_name, get_active_session, get_resurrectable_sessions, get_sessions,
    get_sessions_sorted_by_mtime, kill_session as kill_session_impl, match_session_name,
    print_sessions, print_sessions_with_index, resurrection_layout, session_exists,
    validate_session_name, ActiveSession, SessionNameMatch,
};

use zellij_utils::consts::session_layout_cache_file_name;

#[cfg(feature = "web_server_capability")]
use zellij_client::web_client::start_web_client as start_web_client_impl;

#[cfg(feature = "web_server_capability")]
use zellij_utils::web_server_commands::shutdown_all_webserver_instances;

#[cfg(feature = "web_server_capability")]
use zellij_utils::web_authentication_tokens::{
    create_token, list_tokens, revoke_all_tokens, revoke_token,
};

use miette::{Report, Result};
use serde_json::json;
use zellij_server::{os_input_output::get_server_os_input, start_server as start_server_impl};
use zellij_utils::{
    cli::{CliAction, CliArgs, Command, SessionCommand, Sessions},
    data::{ConnectToSession, ListPanesResponse},
    envs,
    input::{
        actions::Action,
        config::{Config, ConfigError},
        options::Options,
    },
    setup::Setup,
};

pub(crate) use zellij_utils::sessions::list_sessions;

pub(crate) fn kill_all_sessions(yes: bool) {
    match get_sessions() {
        Ok(sessions) if sessions.is_empty() => {
            eprintln!("No active zellij sessions found.");
            process::exit(1);
        },
        Ok(sessions) => {
            if !yes {
                println!("WARNING: this action will kill all sessions.");
                if !Confirm::new()
                    .with_prompt("Do you want to continue?")
                    .interact()
                    .unwrap()
                {
                    println!("Abort.");
                    process::exit(1);
                }
            }
            for session in &sessions {
                kill_session_impl(&session.0);
            }
            process::exit(0);
        },
        Err(e) => {
            eprintln!("Error occurred: {:?}", e);
            process::exit(1);
        },
    }
}

pub(crate) fn delete_all_sessions(yes: bool, force: bool) {
    let active_sessions: Vec<String> = get_sessions()
        .unwrap_or_default()
        .iter()
        .map(|s| s.0.clone())
        .collect();
    let resurrectable_sessions = get_resurrectable_sessions();
    let dead_sessions: Vec<_> = if force {
        resurrectable_sessions
    } else {
        resurrectable_sessions
            .iter()
            .filter(|(name, _)| !active_sessions.contains(name))
            .cloned()
            .collect()
    };
    if !yes {
        println!("WARNING: this action will delete all resurrectable sessions.");
        if !Confirm::new()
            .with_prompt("Do you want to continue?")
            .interact()
            .unwrap()
        {
            println!("Abort.");
            process::exit(1);
        }
    }
    for session in &dead_sessions {
        delete_session_impl(&session.0, force);
    }
    process::exit(0);
}

pub(crate) fn kill_session(target_session: &Option<String>) {
    match target_session {
        Some(target_session) => {
            assert_session(target_session);
            kill_session_impl(target_session);
            process::exit(0);
        },
        None => {
            println!("Please specify the session name to kill.");
            process::exit(1);
        },
    }
}

pub(crate) fn delete_session(target_session: &Option<String>, force: bool) {
    match target_session {
        Some(target_session) => {
            if let Err(e) = validate_session_name(target_session) {
                eprintln!("{}", e);
                process::exit(1);
            }
            assert_dead_session(target_session, force);
            delete_session_impl(target_session, force);
            process::exit(0);
        },
        None => {
            println!("Please specify the session name to delete.");
            process::exit(1);
        },
    }
}

fn get_os_input<OsInputOutput>(
    fn_get_os_input: fn() -> Result<OsInputOutput, std::io::Error>,
) -> OsInputOutput {
    match fn_get_os_input() {
        Ok(os_input) => os_input,
        Err(e) => {
            eprintln!("failed to open terminal:\n{}", e);
            process::exit(1);
        },
    }
}

pub(crate) fn start_server(path: PathBuf, debug: bool) {
    // Set instance-wide debug mode
    zellij_utils::consts::DEBUG_MODE.set(debug).unwrap();
    let os_input = get_os_input(get_server_os_input);
    start_server_impl(Box::new(os_input), path);
}

#[cfg(feature = "web_server_capability")]
pub(crate) fn start_web_server(
    opts: CliArgs,
    run_daemonized: bool,
    ip: Option<IpAddr>,
    port: Option<u16>,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    startup_timeout: Option<u64>,
) {
    // TODO: move this outside of this function
    let (config, _layout, config_options, _config_without_layout, _config_options_without_layout) =
        match Setup::from_cli_args(&opts) {
            Ok(results) => results,
            Err(e) => {
                if let ConfigError::KdlError(error) = e {
                    let report: Report = error.into();
                    eprintln!("{:?}", report);
                } else {
                    eprintln!("{}", e);
                }
                process::exit(1);
            },
        };
    start_web_client_impl(
        config,
        config_options,
        opts.config,
        run_daemonized,
        ip,
        port,
        cert,
        key,
        startup_timeout,
    );
}

#[cfg(not(feature = "web_server_capability"))]
pub(crate) fn start_web_server(
    _opts: CliArgs,
    _run_daemonized: bool,
    _ip: Option<IpAddr>,
    _port: Option<u16>,
    _cert: Option<PathBuf>,
    _key: Option<PathBuf>,
    _startup_timeout: Option<u64>,
) {
    log::error!(
        "This version of Zellij was compiled without web server support, cannot run web server!"
    );
    eprintln!(
        "This version of Zellij was compiled without web server support, cannot run web server!"
    );
    std::process::exit(2);
}

fn create_new_client() -> ClientInfo {
    ClientInfo::New(generate_unique_session_name_or_exit(), None, None)
}

fn attach_create_command(session_name: String) -> Command {
    Command::Sessions(Sessions::Attach {
        session_name: Some(session_name),
        create: true,
        create_background: false,
        index: None,
        options: None,
        force_run_commands: false,
        token: None,
        remember: false,
        forget: false,
        ca_cert: None,
        insecure: false,
    })
}

fn generate_short_room_id() -> String {
    for id in 1..10_000u64 {
        let candidate = id.to_string();
        if room_metadata(&candidate).is_err() && !room_session_is_active(&candidate) {
            return candidate;
        }
    }
    generate_unique_session_name_or_exit()
}

fn choose_room_id_from_context(
    shared_id: Option<String>,
    current_session_name: Option<String>,
) -> String {
    shared_id
        .or(current_session_name)
        .unwrap_or_else(generate_short_room_id)
}

fn current_session_name_from_context() -> Option<String> {
    envs::get_session_name().ok()
}

fn resolve_default_room_id(shared_id: Option<String>) -> String {
    choose_room_id_from_context(shared_id, current_session_name_from_context())
}

fn terminal_provider_command(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "gemini" => Some("gemini"),
        _ => None,
    }
}

/// Generate a wrapper script that sets ACP env vars before launching the provider command.
/// This ensures hooks can detect ZELLIJ_AGENT_SHARED_ID inside the pane.
fn provider_wrapper_script(
    shared_id: &str,
    endpoint_id: &str,
    provider: &str,
    role: &str,
    provider_command: &str,
) -> String {
    let wrapper_dir = crate::agent_control_plane::room_id_root().join(shared_id);
    let _ = std::fs::create_dir_all(&wrapper_dir);
    let wrapper_path = wrapper_dir.join(format!("{}.sh", endpoint_id));
    let script = format!(
        "#!/bin/bash\nexport ZELLIJ_AGENT_SHARED_ID={shared_id}\nexport ZELLIJ_AGENT_ENDPOINT_ID={endpoint_id}\nexport ZELLIJ_AGENT_PROVIDER={provider}\nexport ZELLIJ_AGENT_ROLE={role}\nexec {provider_command} \"$@\"\n"
    );
    let _ = std::fs::write(&wrapper_path, &script);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&wrapper_path, std::fs::Permissions::from_mode(0o755));
    }
    wrapper_path.display().to_string()
}

fn latest_driver_endpoint(endpoints: &[EndpointMetadata]) -> Option<&EndpointMetadata> {
    endpoints.iter().rev().find(|endpoint| {
        endpoint.role == "driver" && terminal_provider_command(&endpoint.provider).is_some()
    })
}

fn reviewer_provider_context(
    endpoints: &[EndpointMetadata],
    fallback_cwd: &Path,
) -> (String, PathBuf) {
    latest_driver_endpoint(endpoints)
        .and_then(|endpoint| {
            terminal_provider_command(&endpoint.provider)
                .map(|provider_command| (provider_command.to_owned(), PathBuf::from(&endpoint.cwd)))
        })
        .unwrap_or_else(|| ("claude".to_owned(), fallback_cwd.to_path_buf()))
}

fn provider_context_for_persona(
    persona: AgentPersona,
    endpoints: &[EndpointMetadata],
    fallback_cwd: &Path,
) -> (Option<String>, PathBuf) {
    match persona {
        AgentPersona::Claude => (Some("claude".to_owned()), fallback_cwd.to_path_buf()),
        AgentPersona::Codex => (Some("codex".to_owned()), fallback_cwd.to_path_buf()),
        AgentPersona::Gemini => (Some("gemini".to_owned()), fallback_cwd.to_path_buf()),
        AgentPersona::Reviewer => {
            let (provider_command, cwd) = reviewer_provider_context(endpoints, fallback_cwd);
            (Some(provider_command), cwd)
        },
    }
}

fn apply_room_driver_provider_args(
    shared_id: &str,
    persona: AgentPersona,
    provider_command: Option<String>,
) -> Option<String> {
    if persona == AgentPersona::Reviewer {
        return provider_command;
    }
    let extra_args = room_metadata(shared_id)
        .ok()
        .and_then(|metadata| metadata.driver_provider_args)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    match (provider_command, extra_args) {
        (Some(provider_command), Some(extra_args)) => Some(format!(
            "{} {}",
            provider_command,
            extra_args
        )),
        (provider_command, None) => provider_command,
        (None, _) => None,
    }
}

fn reviewer_bootstrap_message(endpoint: &EndpointMetadata) -> Option<String> {
    let bootstrap_prompt = endpoint.reviewer_bootstrap_prompt.as_deref()?;
    let review_message_template = endpoint.review_message_template.as_deref()?;
    Some(format!(
        "{bootstrap_prompt}\n\n身份信息:\n- room_id: {room_id}\n- reviewer_endpoint: {endpoint_id}\n- role: reviewer\n- review_target_role: driver\n\n评审目标:\n- 保持客观：基于证据校验 driver 的说法，不确定项要明确标注。\n- 促进收敛：目标是把问题收敛到可交付状态，避免无休止往返。\n\n你回给 driver 时，严格使用以下结构化协议:\n[ACP_REVIEW_RESPONSE_V1]\nroom_id: {room_id}\nrequest_id: <request_id>\nsource_endpoint: {endpoint_id}\ntarget_role: driver\nseverity: <low|medium|high>\nconfidence: <low|medium|high>\nshould_send: true\nloop_control: <CONTINUE|END_REVIEW>\nsummary: <one-line summary>\n\nfindings:\n- <finding>\n\nactions:\n- <action>\n\nnotes:\n- <optional notes>\n[/ACP_REVIEW_RESPONSE_V1]\n\n规则:\n- 把 `request_id: <request_id>` 替换为最新 ACP_REVIEW_REQUEST 中的真实 request_id。\n- 每个 request 只返回一个 ACP_REVIEW_RESPONSE_V1 block。\n- 常规轮次使用 `loop_control: CONTINUE`。\n- 当你判断质量已达标、无需继续迭代时，使用 `loop_control: END_REVIEW`。\n- 如果要给 driver 一条最终总结，就 `should_send: true`；如果想静默收敛，就 `should_send: false`。\n- 返回一轮后等待下一条 request，不要连续输出多轮。\n\n人类可读模板:\n{review_message_template}\n\n首次只回复一次 READY，然后等待 review request。",
        room_id = endpoint.shared_id,
        endpoint_id = endpoint.endpoint_id,
    ))
}

#[derive(Debug, Clone, Copy, Default)]
struct RoomRoleCounts {
    driver_count: usize,
    reviewer_count: usize,
    human_count: usize,
}

const REVIEW_AUTOMATION_SCHEMA_VERSION: u32 = 1;
const REVIEW_AUTOMATION_POLL_MS: u64 = 1200;
const REVIEW_AUTOMATION_IDLE_SECS: u64 = 4;
const REVIEW_REQUEST_TIMEOUT_SECS: u64 = 300;
const REVIEW_RESPONSE_START_MARKER: &str = "[ACP_REVIEW_RESPONSE_V1]";
const REVIEW_RESPONSE_END_MARKER: &str = "[/ACP_REVIEW_RESPONSE_V1]";
const REVIEW_LOOP_CONTROL_END: &str = "END_REVIEW";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReviewAutomationState {
    schema_version: u32,
    auto_review_enabled: bool,
    auto_send_enabled: bool,
    worker_started_at: Option<String>,
    last_driver_fingerprint: Option<String>,
    last_request_id: Option<String>,
    active_request_id: Option<String>,
    active_request_started_at: Option<String>,
    last_response_fingerprint: Option<String>,
    updated_at: String,
}

impl Default for ReviewAutomationState {
    fn default() -> Self {
        Self {
            schema_version: REVIEW_AUTOMATION_SCHEMA_VERSION,
            auto_review_enabled: false,
            auto_send_enabled: false,
            worker_started_at: None,
            last_driver_fingerprint: None,
            last_request_id: None,
            active_request_id: None,
            active_request_started_at: None,
            last_response_fingerprint: None,
            updated_at: timestamp_now(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct StreamEventRow {
    time: String,
    kind: String,
    message: String,
    endpoint_id: Option<String>,
    payload: serde_json::Value,
}

#[derive(Debug, Clone)]
struct DriverTurnState {
    fingerprint: String,
    latest_event_at: SystemTime,
}

fn timestamp_now() -> String {
    format_rfc3339_seconds(SystemTime::now()).to_string()
}

fn review_automation_dir(shared_id: &str) -> PathBuf {
    room_id_root().join(shared_id).join("review")
}

fn review_automation_state_path(shared_id: &str) -> PathBuf {
    review_automation_dir(shared_id).join("automation.json")
}

fn review_automation_worker_lock_path(shared_id: &str) -> PathBuf {
    review_automation_dir(shared_id).join("auto-worker.lock")
}

fn load_review_automation_state(shared_id: &str) -> ReviewAutomationState {
    let state_path = review_automation_state_path(shared_id);
    let Ok(raw) = fs::read_to_string(state_path) else {
        return ReviewAutomationState::default();
    };
    let mut state: ReviewAutomationState =
        serde_json::from_str(&raw).unwrap_or_else(|_| ReviewAutomationState::default());
    if state.schema_version == 0 {
        state.schema_version = REVIEW_AUTOMATION_SCHEMA_VERSION;
    }
    // Backward-compatible migration: old state had only last_request_id.
    // If no response has been recorded yet, treat last_request_id as in-flight.
    if state.active_request_id.is_none()
        && state.last_response_fingerprint.is_none()
        && state.last_request_id.is_some()
    {
        state.active_request_id = state.last_request_id.clone();
        state.active_request_started_at = Some(state.updated_at.clone());
    }
    if state.active_request_id.is_some() && state.active_request_started_at.is_none() {
        state.active_request_started_at = Some(state.updated_at.clone());
    }
    state
}

fn review_request_timed_out(started_at: &str) -> bool {
    parse_rfc3339(started_at)
        .ok()
        .and_then(|started_at| SystemTime::now().duration_since(started_at).ok())
        .map(|elapsed| elapsed >= Duration::from_secs(REVIEW_REQUEST_TIMEOUT_SECS))
        .unwrap_or(false)
}

fn save_review_automation_state(
    shared_id: &str,
    mut state: ReviewAutomationState,
) -> Result<ReviewAutomationState, String> {
    state.schema_version = REVIEW_AUTOMATION_SCHEMA_VERSION;
    state.updated_at = timestamp_now();
    let path = review_automation_state_path(shared_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
    fs::write(path, raw).map_err(|e| e.to_string())?;
    Ok(state)
}

fn stream_jsonl_path(shared_id: &str, stream: ViewStreamKind) -> PathBuf {
    match stream {
        ViewStreamKind::Events => room_id_root().join(shared_id).join("events.jsonl"),
        ViewStreamKind::Code => room_id_root().join(shared_id).join("code.jsonl"),
    }
}

fn read_stream_rows(
    shared_id: &str,
    stream: ViewStreamKind,
) -> Result<Vec<StreamEventRow>, String> {
    let path = stream_jsonl_path(shared_id, stream);
    if !path.exists() {
        return Ok(vec![]);
    }
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<StreamEventRow>(line).map_err(|e| e.to_string()))
        .collect()
}

fn terminal_id_from_pane_id(pane_id: &str) -> Option<u64> {
    pane_id
        .strip_prefix("terminal_")
        .and_then(|id| id.parse::<u64>().ok())
}

fn event_matches_pane(
    event: &StreamEventRow,
    pane_id: &str,
    driver_endpoint_id: Option<&str>,
) -> bool {
    // Match by pane_id in payload
    if event
        .payload
        .get("pane_id")
        .and_then(|value| value.as_str())
        .map(|value| value == pane_id)
        .unwrap_or(false)
    {
        return true;
    }
    // Match by terminal_id in payload
    if let Some(terminal_id) = terminal_id_from_pane_id(pane_id) {
        if event
            .payload
            .get("terminal_id")
            .and_then(|value| value.as_u64())
            .map(|value| value == terminal_id)
            .unwrap_or(false)
        {
            return true;
        }
    }
    // Match by endpoint_id in event (provider hooks don't include pane_id/terminal_id).
    if let Some(driver_endpoint_id) = driver_endpoint_id {
        if event.endpoint_id.as_deref() == Some(driver_endpoint_id) {
            return true;
        }
    }
    false
}

fn driver_relevant_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "write_character"
            | "write_to_pane_id"
            | "paste"
            | "tool_call"
            | "pane_snapshot"
    )
}

fn is_internal_acp_injection_event(event: &StreamEventRow) -> bool {
    event.kind == "paste"
        && (event.message.contains("[ACP_MESSAGE_BEGIN]")
            || event.message.contains("[ACP_REVIEW_REQUEST_BEGIN]"))
}

fn stable_fingerprint(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn event_stable_signature(event: &StreamEventRow) -> String {
    let payload = serde_json::to_string(&event.payload).unwrap_or_default();
    format!("{}|{}|{}", event.kind, event.message, payload)
}

fn driver_turn_state(
    shared_id: &str,
    driver_pane_id: &str,
    driver_endpoint_id: &str,
) -> Result<Option<DriverTurnState>, String> {
    let events = read_stream_rows(shared_id, ViewStreamKind::Events)?;
    let relevant_events: Vec<&StreamEventRow> = events
        .iter()
        .filter(|event| event_matches_pane(event, driver_pane_id, Some(driver_endpoint_id)))
        .filter(|event| driver_relevant_event_kind(&event.kind))
        .filter(|event| !is_internal_acp_injection_event(event))
        .collect();
    if relevant_events.is_empty() {
        return Ok(None);
    }
    let mut last_signature: Option<String> = None;
    let mut latest = relevant_events
        .first()
        .and_then(|event| parse_rfc3339(&event.time).ok())
        .unwrap_or_else(SystemTime::now);
    let mut stable_events = Vec::new();
    for event in &relevant_events {
        let signature = event_stable_signature(event);
        if last_signature.as_deref() == Some(signature.as_str()) {
            continue;
        }
        last_signature = Some(signature.clone());
        latest = parse_rfc3339(&event.time).unwrap_or_else(|_| SystemTime::now());
        stable_events.push(signature);
    }
    let fingerprint_source = stable_events
        .iter()
        .rev()
        .take(24)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(DriverTurnState {
        fingerprint: stable_fingerprint(&fingerprint_source),
        latest_event_at: latest,
    }))
}

fn extract_latest_response_block_from_text(text: &str) -> Option<String> {
    const LEGACY_START: &str = "[ACP_REVIEW_RESPONSE_V1_BEGIN]";
    const LEGACY_END: &str = "[ACP_REVIEW_RESPONSE_V1_END]";

    let marker_pairs = [
        (REVIEW_RESPONSE_START_MARKER, REVIEW_RESPONSE_END_MARKER),
        (LEGACY_START, LEGACY_END),
    ];
    for (start_marker, end_marker) in marker_pairs {
        if let Some(end_index) = text.rfind(end_marker) {
            let prefix = &text[..end_index];
            if let Some(start_index) = prefix.rfind(start_marker) {
                let end = end_index + end_marker.len();
                return Some(text[start_index..end].trim().to_owned());
            }
        }
    }
    None
}

fn latest_reviewer_response_block(
    shared_id: &str,
    reviewer_pane_ids: &HashSet<String>,
) -> Result<Option<(String, String, String)>, String> {
    let events = read_stream_rows(shared_id, ViewStreamKind::Events)?;
    for event in events.iter().rev() {
        if event.kind != "pane_snapshot" {
            continue;
        }
        let pane_id = event
            .payload
            .get("pane_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !reviewer_pane_ids.contains(pane_id) {
            continue;
        }
        let lines = event
            .payload
            .get("snapshot")
            .or_else(|| event.payload.get("viewport"))
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let snapshot = lines
            .iter()
            .filter_map(|line| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(block) = extract_latest_response_block_from_text(&snapshot) {
            return Ok(Some((block, pane_id.to_owned(), event.time.clone())));
        }
    }
    Ok(None)
}

fn snapshot_is_fresh_for_request(snapshot_time: &str, request_started_at: Option<&str>) -> bool {
    let Some(request_started_at) = request_started_at else {
        return true;
    };
    let Ok(snapshot_time) = parse_rfc3339(snapshot_time) else {
        return true;
    };
    let Ok(request_started_at) = parse_rfc3339(request_started_at) else {
        return true;
    };
    snapshot_time >= request_started_at
}

fn parse_review_response_request_id(raw_block: &str) -> Option<String> {
    extract_latest_response_block_from_text(raw_block).and_then(|block| {
        block.lines().find_map(|line| {
            line.trim()
                .strip_prefix("request_id:")
                .map(|value| value.trim().to_owned())
        })
    })
}

fn is_placeholder_request_id(request_id: &str) -> bool {
    matches!(request_id.trim(), "<request_id>" | "{request_id}" | "request_id")
}

fn reviewer_response_is_unfilled_template(raw_block: &str) -> bool {
    let markers = [
        "request_id: <request_id>",
        "severity: <low|medium|high>",
        "confidence: <low|medium|high>",
        "summary: <one-line summary>",
        "- <finding>",
        "- <action>",
        "- <optional notes>",
    ];
    let hit_count = markers
        .iter()
        .filter(|marker| raw_block.contains(**marker))
        .count();
    hit_count >= 2
}

fn normalize_review_response_request_id(raw_block: &str, request_id: &str) -> String {
    raw_block
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("request_id:") {
                format!("request_id: {}", request_id)
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_review_response_loop_control(raw_block: &str) -> Option<String> {
    extract_latest_response_block_from_text(raw_block).and_then(|block| {
        block.lines().find_map(|line| {
            line.trim()
                .strip_prefix("loop_control:")
                .map(|value| value.trim().to_ascii_uppercase())
        })
    })
}

fn review_response_requests_loop_end(raw_block: &str) -> bool {
    parse_review_response_loop_control(raw_block)
        .map(|value| {
            matches!(
                value.as_str(),
                REVIEW_LOOP_CONTROL_END | "END" | "DONE" | "CONVERGED" | "FINISH" | "FINISHED"
            )
        })
        .unwrap_or(false)
}

fn kdl_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn room_tab_label(shared_id: &str) -> String {
    format!("ACP {}", shared_id)
}

fn count_room_roles(endpoints: &[EndpointMetadata]) -> RoomRoleCounts {
    let mut counts = RoomRoleCounts::default();
    for endpoint in endpoints {
        match endpoint.role.as_str() {
            "driver" => counts.driver_count += 1,
            "reviewer" => counts.reviewer_count += 1,
            "human" => counts.human_count += 1,
            _ => {},
        }
    }
    counts
}

fn send_acp_room_state_to_session(
    session_name: &str,
    shared_id: &str,
) -> std::result::Result<(), String> {
    let endpoints = list_room_endpoints(shared_id).map_err(|e| e.to_string())?;
    let automation_state = load_review_automation_state(shared_id);
    let counts = count_room_roles(&endpoints);
    let payload = serde_json::to_string(&json!({
        "room_id": shared_id,
        "driver_count": counts.driver_count,
        "reviewer_count": counts.reviewer_count,
        "human_count": counts.human_count,
        "auto_review_enabled": automation_state.auto_review_enabled,
        "auto_send_enabled": automation_state.auto_send_enabled,
    }))
    .map_err(|e| e.to_string())?;
    run_cli_action_in_session(
        session_name,
        CliAction::Pipe {
            name: Some("acp-room-state".to_owned()),
            payload: Some(payload),
            args: None,
            plugin: None,
            plugin_configuration: None,
            force_launch_plugin: false,
            skip_plugin_cache: false,
            floating_plugin: None,
            in_place_plugin: None,
            plugin_cwd: None,
            plugin_title: None,
        },
    )
}

fn room_layout_string(
    shared_id: &str,
    provider_command: Option<&str>,
    cwd: &PathBuf,
    endpoint: &EndpointMetadata,
    role_counts: RoomRoleCounts,
) -> String {
    let automation_state = load_review_automation_state(&endpoint.shared_id);
    let shared_id_label = room_tab_label(shared_id);
    let shared_id = kdl_string(shared_id);
    let tab_label = kdl_string(&shared_id_label);
    let cwd = kdl_string(&cwd.display().to_string());
    let self_provider = kdl_string(&endpoint.provider);
    let self_role = kdl_string(&endpoint.role);
    let auto_review_enabled = kdl_string(if automation_state.auto_review_enabled {
        "true"
    } else {
        "false"
    });
    let auto_send_enabled = kdl_string(if automation_state.auto_send_enabled {
        "true"
    } else {
        "false"
    });
    let acp_bar = format!(
        "        pane size=1 borderless=true {{\n            plugin location=\"zellij:acp-bar\" {{\n                room_id {shared_id}\n                self_provider {self_provider}\n                self_role {self_role}\n                driver_count {}\n                reviewer_count {}\n                human_count {}\n                auto_review_enabled {auto_review_enabled}\n                auto_send_enabled {auto_send_enabled}\n            }}\n        }}",
        role_counts.driver_count, role_counts.reviewer_count, role_counts.human_count
    );
    match provider_command {
        Some(provider_command) => {
            let command = kdl_string(provider_command);
            let pane_name = kdl_string(endpoint.label.as_str());
            format!(
                "layout {{\n    tab name={tab_label} focus=true {{\n        pane size=1 borderless=true {{\n            plugin location=\"tab-bar\"\n        }}\n        pane command={command} cwd={cwd} name={pane_name} focus=true\n        pane size=1 borderless=true {{\n            plugin location=\"status-bar\"\n        }}\n{acp_bar}\n    }}\n}}"
            )
        },
        None => format!(
            "layout {{\n    tab name={tab_label} focus=true {{\n        pane size=1 borderless=true {{\n            plugin location=\"tab-bar\"\n        }}\n        pane cwd={cwd} focus=true\n        pane size=1 borderless=true {{\n            plugin location=\"status-bar\"\n        }}\n{acp_bar}\n    }}\n}}"
        ),
    }
}

fn spawn_provider_pane(
    session_name: &str,
    provider_command: &str,
    cwd: PathBuf,
    pane_name: Option<String>,
    in_place: bool,
) -> Result<String, String> {
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = process::Command::new(current_exe);
    command
        .arg("--session")
        .arg(session_name)
        .arg("action")
        .arg("new-pane")
        .arg("--cwd")
        .arg(&cwd);
    if in_place {
        command.arg("--in-place").arg("--close-replaced-pane");
    }
    if let Some(pane_name) = pane_name {
        command.arg("--name").arg(pane_name);
    }
    command.arg("--").arg(provider_command);
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("new-pane failed with status {}", output.status)
        } else {
            stderr
        });
    }
    let pane_id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if pane_id.is_empty() {
        Err(format!(
            "new-pane did not return a pane id for session '{}'",
            session_name
        ))
    } else {
        Ok(pane_id)
    }
}

fn ensure_code_view_pane_in_session(
    session_name: &str,
    shared_id: &str,
    cwd: &Path,
) -> Result<(), String> {
    let command_marker = format!("view code {}", shared_id);
    let title_marker = format!("acp code {}", shared_id).to_lowercase();
    let pane_exists = list_session_panes(session_name)?
        .iter()
        .filter(|entry| !entry.pane_info.is_plugin && !entry.pane_info.is_suppressed)
        .any(|entry| {
            entry
                .pane_command
                .as_deref()
                .map(|command| command.contains(&command_marker))
                .unwrap_or(false)
                || entry
                    .pane_info
                    .title
                    .to_lowercase()
                    .contains(&title_marker)
        });
    if pane_exists {
        return Ok(());
    }

    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let output = process::Command::new(current_exe)
        .arg("--session")
        .arg(session_name)
        .arg("action")
        .arg("new-pane")
        .arg("--cwd")
        .arg(cwd)
        .arg("--name")
        .arg(format!("acp code {}", shared_id))
        .arg("--")
        .arg("zellij")
        .arg("view")
        .arg("code")
        .arg(shared_id)
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            format!(
                "failed to spawn code view pane in session '{}' with status {}",
                session_name, output.status
            )
        } else {
            stderr
        })
    }
}

fn load_reviewer_prompt_override(
    reviewer_prompt: Option<String>,
    reviewer_prompt_file: Option<PathBuf>,
) -> Result<Option<String>, String> {
    let content = if let Some(prompt) = reviewer_prompt {
        prompt
    } else if let Some(path) = reviewer_prompt_file {
        fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read reviewer prompt file '{}': {}", path.display(), e))?
    } else {
        return Ok(None);
    };
    let normalized = content.trim().to_owned();
    if normalized.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized))
    }
}

fn inject_reviewer_bootstrap_into_pane(
    session_name: &str,
    pane_id: &str,
    endpoint: &EndpointMetadata,
) -> Result<(), String> {
    let Some(bootstrap_message) = reviewer_bootstrap_message(endpoint) else {
        return Ok(());
    };
    thread::sleep(Duration::from_millis(250));
    run_cli_action_in_session(
        session_name,
        CliAction::Paste {
            chars: bootstrap_message,
            pane_id: Some(pane_id.to_owned()),
        },
    )?;
    run_cli_action_in_session(
        session_name,
        CliAction::SendKeys {
            keys: vec!["Enter".to_owned()],
            pane_id: Some(pane_id.to_owned()),
        },
    )?;
    let _ = append_stream_event(
        ViewStreamKind::Events,
        &endpoint.shared_id,
        "reviewer_bootstrap_injected",
        Some(endpoint),
        format!(
            "Injected reviewer bootstrap into {} for {}",
            pane_id, endpoint.endpoint_id
        ),
        json!({
            "target_pane_id": pane_id,
        }),
    );
    Ok(())
}

pub(crate) fn start_shared_room(
    mut opts: CliArgs,
    persona: AgentPersona,
    shared_id: Option<String>,
    reviewer_count: Option<usize>,
    reviewer_prompt: Option<String>,
    reviewer_prompt_file: Option<PathBuf>,
    provider_args: Option<String>,
) {
    let shared_id = resolve_default_room_id(shared_id);
    let current_session_name = envs::get_session_name().ok();
    let launched_inside_target_session =
        current_session_name.as_deref() == Some(shared_id.as_str());
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let reviewer_prompt_input_provided = reviewer_prompt.is_some() || reviewer_prompt_file.is_some();
    let reviewer_prompt_override =
        load_reviewer_prompt_override(reviewer_prompt, reviewer_prompt_file).unwrap_or_else(|e| {
            eprintln!("{}", e);
            process::exit(2);
        });
    crate::agent_control_plane::ensure_room(&shared_id).unwrap_or_else(|e| {
        eprintln!("Failed to initialize shared room '{}': {}", shared_id, e);
        process::exit(2);
    });
    if let Some(reviewer_count) = reviewer_count {
        if reviewer_count == 0 {
            eprintln!("--reviewer-count must be >= 1.");
            process::exit(2);
        }
        if let Err(e) = set_room_reviewer_target_count(&shared_id, reviewer_count) {
            eprintln!(
                "Failed to persist reviewer_count for shared room '{}': {}",
                shared_id, e
            );
            process::exit(2);
        }
    }
    if reviewer_prompt_input_provided {
        if let Err(e) = set_room_reviewer_prompt_override(&shared_id, reviewer_prompt_override.clone()) {
            eprintln!(
                "Failed to persist reviewer prompt for shared room '{}': {}",
                shared_id, e
            );
            process::exit(2);
        }
    }
    if provider_args.is_some() {
        if let Err(e) = set_room_driver_provider_args(&shared_id, provider_args.clone()) {
            eprintln!(
                "Failed to persist provider args for shared room '{}': {}",
                shared_id, e
            );
            process::exit(2);
        }
    }
    let participant = match register_participant(&shared_id, persona, &current_dir) {
        Ok(participant) => participant,
        Err(e) => {
            eprintln!("Failed to register shared room '{}': {}", shared_id, e);
            process::exit(2);
        },
    };
    // Auto-configure provider hooks for ACP integration
    ensure_provider_hooks(persona);
    let _ = append_stream_event(
        ViewStreamKind::Events,
        &shared_id,
        "room_ready",
        Some(&participant.endpoint),
        format!(
            "{} joined room '{}' as {}",
            participant.endpoint.provider, shared_id, participant.endpoint.role
        ),
        json!({
            "cwd": participant.endpoint.cwd,
            "note": "Live file and command mirroring will append here in later bridge stages.",
        }),
    );

    std::env::set_var("ZELLIJ_AGENT_SHARED_ID", &shared_id);
    std::env::set_var(
        "ZELLIJ_AGENT_ENDPOINT_ID",
        &participant.endpoint.endpoint_id,
    );
    std::env::set_var("ZELLIJ_AGENT_PROVIDER", &participant.endpoint.provider);
    std::env::set_var("ZELLIJ_AGENT_ROLE", &participant.endpoint.role);
    let room_endpoints = list_room_endpoints(&shared_id).unwrap_or_default();
    let room_role_counts = count_room_roles(&room_endpoints);

    if !launched_inside_target_session {
        println!(
            "Shared room: {} ({})",
            shared_id,
            if participant.room_created {
                "created"
            } else {
                "joined"
            }
        );
        println!("Session name: {}", participant.session_name);
        println!(
            "Endpoint: {} [{} / {}]",
            participant.endpoint.label, participant.endpoint.provider, participant.endpoint.role
        );
        if persona == AgentPersona::Reviewer {
            println!("");
            println!("Reviewer bootstrap:");
            println!(
                "{}",
                participant
                    .endpoint
                    .reviewer_bootstrap_prompt
                    .as_deref()
                    .unwrap_or_default()
            );
            println!("");
            println!("Review message template:");
            println!(
                "{}",
                participant
                    .endpoint
                    .review_message_template
                    .as_deref()
                    .unwrap_or_default()
            );
        } else {
            println!(
                "Share this room with another participant using: zellij reviewer {}",
                shared_id
            );
        }
        if reviewer_count.is_some() || reviewer_prompt_input_provided || provider_args.is_some() {
            println!("");
            if let Some(reviewer_count) = reviewer_count {
                println!("Workspace reviewer count: {}", reviewer_count);
            }
            if reviewer_prompt_input_provided {
                println!(
                    "Workspace reviewer prompt: {}",
                    if reviewer_prompt_override.is_some() {
                        "customized"
                    } else {
                        "cleared (default prompt)"
                    }
                );
            }
            if let Some(provider_args) = provider_args.as_deref() {
                println!("Workspace provider args: {}", provider_args);
            }
        }
    }

    let (provider_command, provider_cwd) =
        provider_context_for_persona(persona, &room_endpoints, &current_dir);
    let provider_command =
        apply_room_driver_provider_args(&shared_id, persona, provider_command);
    let session_active = room_session_is_active(&shared_id);

    // Auto-start review automation before any early return
    let _ = auto_enable_review_automation(&shared_id);

    // Wrap provider command with ACP env vars so hooks work inside provider panes.
    let wrapped_provider_command = provider_command.as_deref().map(|cmd| {
        provider_wrapper_script(
            &shared_id,
            &participant.endpoint.endpoint_id,
            &participant.endpoint.provider,
            &participant.endpoint.role,
            cmd,
        )
    });

    if participant.room_created || !session_active {
        opts.command = None;
        opts.session = if launched_inside_target_session {
            None
        } else {
            Some(shared_id.clone())
        };
        opts.layout = None;
        opts.new_session_with_layout = None;
        let layout = room_layout_string(
            &shared_id,
            wrapped_provider_command.as_deref(),
            &provider_cwd,
            &participant.endpoint,
            room_role_counts,
        );
        opts.layout_string = Some(layout);
        start_client(opts);
        return;
    }

    if let Some(provider_command) = wrapped_provider_command.as_deref() {
        match spawn_provider_pane(
            &shared_id,
            provider_command,
            provider_cwd.clone(),
            Some(participant.endpoint.label.clone()),
            launched_inside_target_session,
        ) {
            Ok(pane_id) => {
                if let Err(e) = update_endpoint_pane_binding(
                    &shared_id,
                    &participant.endpoint.endpoint_id,
                    &pane_id,
                ) {
                    eprintln!(
                        "Failed to bind {} to {} in shared room '{}': {}",
                        participant.endpoint.endpoint_id, pane_id, shared_id, e
                    );
                }
                if persona == AgentPersona::Reviewer {
                    if let Err(e) = inject_reviewer_bootstrap_into_pane(
                        &shared_id,
                        &pane_id,
                        &participant.endpoint,
                    ) {
                        eprintln!(
                            "Failed to inject reviewer bootstrap into '{}' in shared room '{}': {}",
                            pane_id, shared_id, e
                        );
                    }
                }
            },
            Err(e) => {
                eprintln!(
                    "Failed to start {} pane in shared room '{}': {}",
                    provider_command, shared_id, e
                );
            },
        }
    }

    if let Err(e) = ensure_code_view_pane_in_session(&shared_id, &shared_id, &provider_cwd) {
        let _ = append_stream_event(
            ViewStreamKind::Events,
            &shared_id,
            "code_view_pane_spawn_failed",
            Some(&participant.endpoint),
            format!("Failed to auto-open code view pane in '{}'", shared_id),
            json!({
                "error": e,
            }),
        );
    }

    if let Err(e) = send_acp_room_state_to_session(&shared_id, &shared_id) {
        let _ = append_stream_event(
            ViewStreamKind::Events,
            &shared_id,
            "acp_bar_update_failed",
            Some(&participant.endpoint),
            format!("Failed to update ACP bar in '{}'", shared_id),
            json!({
                "error": e,
            }),
        );
    }

    if launched_inside_target_session {
        return;
    }

    opts.session = None;
    opts.layout = None;
    opts.new_session_with_layout = None;
    opts.layout_string = None;
    opts.command = Some(attach_create_command(shared_id));
    start_client(opts);
}

pub(crate) fn view_shared_room(stream: ViewStreamKind, shared_id: Option<String>) {
    let shared_id = resolve_shared_room_id(shared_id);
    if !crate::agent_control_plane::room_exists(&shared_id) {
        eprintln!("Room '{}' does not exist.", shared_id);
        process::exit(2);
    }
    let metadata = room_metadata(&shared_id).unwrap_or_else(|e| {
        eprintln!("Failed to read room metadata for '{}': {}", shared_id, e);
        process::exit(2);
    });
    let endpoints = list_room_endpoints(&shared_id).unwrap_or_else(|e| {
        eprintln!("Failed to read endpoints for '{}': {}", shared_id, e);
        process::exit(2);
    });
    println!(
        "Shared room: {} | session: {} | stream: {}",
        metadata.shared_id,
        metadata.session_name,
        match stream {
            ViewStreamKind::Events => "events",
            ViewStreamKind::Code => "code",
        }
    );
    println!(
        "Transport: {} | network: {}",
        metadata.bridge_transport, metadata.network_transport
    );
    println!(
        "Workspace: reviewer_target={} | reviewer_prompt={} | provider_args={}",
        metadata.reviewer_target_count.max(1),
        if metadata.reviewer_prompt_override.is_some() {
            "custom"
        } else {
            "default"
        },
        metadata
            .driver_provider_args
            .as_deref()
            .unwrap_or("<none>")
    );
    println!("Participants:");
    for endpoint in endpoints {
        println!(
            "- {} [{} / {}] {}",
            endpoint.label, endpoint.provider, endpoint.role, endpoint.cwd
        );
    }
    println!("");
    println!("Following stream. Press Ctrl-C to stop.");
    if let Err(e) = follow_stream(&shared_id, stream) {
        eprintln!("Failed to follow stream for '{}': {}", shared_id, e);
        process::exit(2);
    }
}

fn resolve_shared_room_id(shared_id: Option<String>) -> String {
    shared_id
        .or_else(|| envs::get_session_name().ok())
        .unwrap_or_else(|| {
            eprintln!(
                "Please provide a shared room ID, or run this command inside a Zellij session."
            );
            process::exit(2);
        })
}

fn format_terminal_pane_id(id: u32, is_plugin: bool) -> String {
    if is_plugin {
        format!("plugin_{}", id)
    } else {
        format!("terminal_{}", id)
    }
}

fn room_session_is_active(session_name: &str) -> bool {
    get_sessions()
        .unwrap_or_default()
        .iter()
        .any(|session| session.0 == session_name)
}

fn list_session_panes(session_name: &str) -> std::result::Result<ListPanesResponse, String> {
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let output = process::Command::new(current_exe)
        .arg("--session")
        .arg(session_name)
        .arg("action")
        .arg("list-panes")
        .arg("--json")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            Err(format!("list-panes failed with status {}", output.status))
        } else {
            Err(stderr)
        }
    } else {
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
    }
}

fn resolve_driver_delivery_pane(
    session_name: &str,
    preferred_pane_id: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    let panes = list_session_panes(session_name)?;
    if let Some(preferred_pane_id) = preferred_pane_id {
        if panes.iter().any(|entry| {
            format_terminal_pane_id(entry.pane_info.id, entry.pane_info.is_plugin)
                == preferred_pane_id
        }) {
            return Ok(Some(preferred_pane_id.to_owned()));
        }
    }
    let terminal_panes: Vec<String> = panes
        .iter()
        .filter(|entry| !entry.pane_info.is_plugin && !entry.pane_info.is_suppressed)
        .map(|entry| format_terminal_pane_id(entry.pane_info.id, false))
        .collect();
    if terminal_panes.len() == 1 {
        Ok(terminal_panes.into_iter().next())
    } else {
        Ok(None)
    }
}

fn run_cli_action_in_session(
    session_name: &str,
    cli_action: CliAction,
) -> std::result::Result<(), String> {
    let os_input = get_os_input(zellij_client::os_input_output::get_cli_client_os_input);
    let get_current_dir = || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let actions = Action::actions_from_cli(cli_action, Box::new(get_current_dir), None)?;
    zellij_client::cli_client::start_cli_client(Box::new(os_input), session_name, actions);
    Ok(())
}

fn inject_review_feedback_into_driver(
    shared_id: &str,
    pane_id: &str,
    envelope: &str,
    request_id: &str,
    source_endpoint: &str,
) -> std::result::Result<(), String> {
    run_cli_action_in_session(
        shared_id,
        CliAction::Paste {
            chars: envelope.to_owned(),
            pane_id: Some(pane_id.to_owned()),
        },
    )?;
    run_cli_action_in_session(
        shared_id,
        CliAction::SendKeys {
            keys: vec!["Enter".to_owned()],
            pane_id: Some(pane_id.to_owned()),
        },
    )?;
    let _ = append_stream_event(
        ViewStreamKind::Events,
        shared_id,
        "review_feedback_injected_to_driver",
        None,
        format!("Injected review feedback {} into {}", request_id, pane_id),
        json!({
            "request_id": request_id,
            "target_pane_id": pane_id,
            "source_endpoint": source_endpoint,
        }),
    );
    Ok(())
}

fn resolve_live_bound_reviewer_panes(
    session_name: &str,
    shared_id: &str,
) -> std::result::Result<Vec<(EndpointMetadata, String)>, String> {
    let live_pane_ids: HashSet<String> = list_session_panes(session_name)?
        .iter()
        .filter(|entry| !entry.pane_info.is_plugin && !entry.pane_info.is_suppressed)
        .map(|entry| format_terminal_pane_id(entry.pane_info.id, false))
        .collect();
    let endpoints = list_room_endpoints(shared_id).map_err(|e| e.to_string())?;
    Ok(endpoints
        .into_iter()
        .filter(|endpoint| endpoint.role == "reviewer")
        .filter_map(|endpoint| {
            endpoint.bound_pane_id.clone().and_then(|pane_id| {
                if live_pane_ids.contains(&pane_id) {
                    Some((endpoint, pane_id))
                } else {
                    None
                }
            })
        })
        .collect())
}

fn backfill_driver_pane_binding_from_live_room(
    session_name: &str,
    shared_id: &str,
) -> std::result::Result<(), String> {
    let endpoints = list_room_endpoints(shared_id).map_err(|e| e.to_string())?;
    let drivers: Vec<&EndpointMetadata> = endpoints
        .iter()
        .filter(|endpoint| endpoint.role == "driver")
        .collect();
    if drivers.len() != 1 {
        return Ok(());
    }
    let driver_endpoint = drivers[0];
    let non_driver_bound_pane_ids: HashSet<String> = endpoints
        .iter()
        .filter(|endpoint| endpoint.role != "driver")
        .filter_map(|endpoint| endpoint.bound_pane_id.clone())
        .collect();
    let driver_candidate_panes: Vec<String> = list_session_panes(session_name)?
        .iter()
        .filter(|entry| !entry.pane_info.is_plugin && !entry.pane_info.is_suppressed)
        .map(|entry| format_terminal_pane_id(entry.pane_info.id, false))
        .filter(|pane_id| !non_driver_bound_pane_ids.contains(pane_id))
        .collect();
    if driver_candidate_panes.len() != 1 {
        return Ok(());
    }
    let pane_id = &driver_candidate_panes[0];
    if driver_endpoint.bound_pane_id.as_deref() == Some(pane_id) {
        return Ok(());
    }
    update_endpoint_pane_binding(shared_id, &driver_endpoint.endpoint_id, pane_id)
        .map_err(|e| e.to_string())?;
    let _ = append_stream_event(
        ViewStreamKind::Events,
        shared_id,
        "driver_pane_binding_backfilled",
        Some(driver_endpoint),
        format!(
            "Backfilled driver pane binding for {} -> {}",
            driver_endpoint.endpoint_id, pane_id
        ),
        json!({
            "target_pane_id": pane_id,
        }),
    );
    Ok(())
}

fn inject_review_request_into_reviewer(
    shared_id: &str,
    pane_id: &str,
    endpoint: &EndpointMetadata,
    request_id: &str,
    request_text: &str,
) -> std::result::Result<(), String> {
    run_cli_action_in_session(
        shared_id,
        CliAction::Paste {
            chars: request_text.to_owned(),
            pane_id: Some(pane_id.to_owned()),
        },
    )?;
    run_cli_action_in_session(
        shared_id,
        CliAction::SendKeys {
            keys: vec!["Enter".to_owned()],
            pane_id: Some(pane_id.to_owned()),
        },
    )?;
    let _ = append_stream_event(
        ViewStreamKind::Events,
        shared_id,
        "review_request_injected_to_reviewer",
        Some(endpoint),
        format!("Injected review request {} into {}", request_id, pane_id),
        json!({
            "request_id": request_id,
            "target_pane_id": pane_id,
        }),
    );
    Ok(())
}

fn dispatch_review_request_to_reviewers(
    shared_id: &str,
    persisted: &PersistedReviewRequest,
    print_output: bool,
) {
    if !room_session_is_active(shared_id) {
        return;
    }
    match resolve_live_bound_reviewer_panes(shared_id, shared_id) {
        Ok(reviewer_targets) if reviewer_targets.is_empty() => {
            let _ = append_stream_event(
                ViewStreamKind::Events,
                shared_id,
                "review_request_waiting_for_reviewer_delivery",
                None,
                format!(
                    "Review request {} is waiting for reviewer delivery",
                    persisted.request.request_id
                ),
                json!({
                    "request_id": persisted.request.request_id,
                    "reason": "no_live_bound_reviewer_panes",
                }),
            );
            if print_output {
                println!("No live reviewer panes were found. Request kept on disk.");
            }
        },
        Ok(reviewer_targets) => {
            for (endpoint, pane_id) in reviewer_targets {
                match inject_review_request_into_reviewer(
                    shared_id,
                    &pane_id,
                    &endpoint,
                    &persisted.request.request_id,
                    &persisted.text,
                ) {
                    Ok(()) => {
                        if print_output {
                            println!(
                                "Injected review request into reviewer {} ({})",
                                endpoint.endpoint_id, pane_id
                            );
                        }
                    },
                    Err(e) => {
                        let _ = append_stream_event(
                            ViewStreamKind::Events,
                            shared_id,
                            "review_request_waiting_for_reviewer_delivery",
                            Some(&endpoint),
                            format!(
                                "Review request {} is waiting for reviewer delivery",
                                persisted.request.request_id
                            ),
                            json!({
                                "request_id": persisted.request.request_id,
                                "target_pane_id": pane_id,
                                "reason": "reviewer_injection_failed",
                                "error": e,
                            }),
                        );
                        if print_output {
                            println!(
                                "Could not inject request into reviewer {} ({}). Request kept on disk.",
                                endpoint.endpoint_id, pane_id
                            );
                        }
                    },
                }
            }
        },
        Err(e) => {
            let _ = append_stream_event(
                ViewStreamKind::Events,
                shared_id,
                "review_request_waiting_for_reviewer_delivery",
                None,
                format!(
                    "Review request {} is waiting for reviewer delivery",
                    persisted.request.request_id
                ),
                json!({
                    "request_id": persisted.request.request_id,
                    "reason": "reviewer_pane_query_failed",
                    "error": e,
                }),
            );
            if print_output {
                println!("Could not inspect reviewer panes yet. Request kept on disk.");
            }
        },
    }
}

fn dispatch_persisted_feedback_to_driver(
    shared_id: &str,
    persisted: &PersistedReviewFeedback,
    auto_send_enabled: bool,
    print_output: bool,
) {
    let Some(driver_envelope) = persisted.driver_envelope.as_deref() else {
        return;
    };
    if !auto_send_enabled {
        let _ = append_stream_event(
            ViewStreamKind::Events,
            shared_id,
            "review_feedback_waiting_for_driver_delivery",
            None,
            format!(
                "Feedback {} is waiting for driver delivery",
                persisted.feedback.request_id
            ),
            json!({
                "request_id": persisted.feedback.request_id,
                "target_pane_id": persisted.target_pane_id,
                "reason": "auto_send_disabled",
            }),
        );
        if print_output {
            println!("Auto-send is disabled. Envelope kept on disk.");
        }
        return;
    }
    if room_session_is_active(shared_id) {
        match resolve_driver_delivery_pane(shared_id, persisted.target_pane_id.as_deref()) {
            Ok(Some(target_pane_id)) => {
                if let Err(e) = inject_review_feedback_into_driver(
                    shared_id,
                    &target_pane_id,
                    driver_envelope,
                    &persisted.feedback.request_id,
                    &persisted.feedback.source_endpoint,
                ) {
                    let _ = append_stream_event(
                        ViewStreamKind::Events,
                        shared_id,
                        "review_feedback_waiting_for_driver_delivery",
                        None,
                        format!(
                            "Feedback {} is waiting for driver delivery",
                            persisted.feedback.request_id
                        ),
                        json!({
                            "request_id": persisted.feedback.request_id,
                            "target_pane_id": persisted.target_pane_id,
                            "reason": "driver_injection_failed",
                            "error": e,
                        }),
                    );
                    if print_output {
                        println!("Failed to inject driver envelope. Envelope kept on disk.");
                    }
                } else if print_output {
                    println!("Injected driver envelope into: {}", target_pane_id);
                }
            },
            Ok(None) => {
                let _ = append_stream_event(
                    ViewStreamKind::Events,
                    shared_id,
                    "review_feedback_waiting_for_driver_delivery",
                    None,
                    format!(
                        "Feedback {} is waiting for driver delivery",
                        persisted.feedback.request_id
                    ),
                    json!({
                        "request_id": persisted.feedback.request_id,
                        "target_pane_id": persisted.target_pane_id,
                        "reason": "target_pane_not_resolved",
                    }),
                );
                if print_output {
                    println!(
                        "Driver pane not resolved. Envelope kept on disk for manual delivery."
                    );
                }
            },
            Err(e) => {
                let _ = append_stream_event(
                    ViewStreamKind::Events,
                    shared_id,
                    "review_feedback_waiting_for_driver_delivery",
                    None,
                    format!(
                        "Feedback {} is waiting for driver delivery",
                        persisted.feedback.request_id
                    ),
                    json!({
                        "request_id": persisted.feedback.request_id,
                        "target_pane_id": persisted.target_pane_id,
                        "reason": "pane_query_failed",
                        "error": e,
                    }),
                );
                if print_output {
                    println!("Could not inspect session panes yet. Envelope kept on disk.");
                }
            },
        }
    } else {
        let _ = append_stream_event(
            ViewStreamKind::Events,
            shared_id,
            "review_feedback_waiting_for_driver_delivery",
            None,
            format!(
                "Feedback {} is waiting for driver delivery",
                persisted.feedback.request_id
            ),
            json!({
                "request_id": persisted.feedback.request_id,
                "target_pane_id": persisted.target_pane_id,
                "reason": "session_not_active",
            }),
        );
        if print_output {
            println!("Room session is not active. Envelope kept on disk.");
        }
    }
}

pub(crate) fn prepare_room_review_request(shared_id: Option<String>) {
    let shared_id = resolve_shared_room_id(shared_id);
    if room_session_is_active(&shared_id) {
        let _ = backfill_driver_pane_binding_from_live_room(&shared_id, &shared_id);
    }
    let persisted = prepare_review_request(&shared_id).unwrap_or_else(|e| {
        eprintln!(
            "Failed to prepare review request for shared room '{}': {}",
            shared_id, e
        );
        process::exit(2);
    });
    println!("Request ID: {}", persisted.request.request_id);
    if let Some(driver_endpoint) = persisted.request.driver_endpoint.as_deref() {
        println!("Driver endpoint: {}", driver_endpoint);
    }
    if let Some(target_pane_id) = persisted.request.target_pane_id.as_deref() {
        println!("Target pane: {}", target_pane_id);
    }
    println!("{}", persisted.text);
    println!("Saved request JSON: {}", persisted.json_path.display());
    println!("Saved request text: {}", persisted.text_path.display());
    dispatch_review_request_to_reviewers(&shared_id, &persisted, true);
}

pub(crate) fn record_room_review_feedback(
    shared_id: Option<String>,
    file: Option<PathBuf>,
    source_endpoint: Option<String>,
) {
    let shared_id = resolve_shared_room_id(shared_id);
    let mut raw_feedback = String::new();
    if let Some(file) = file {
        raw_feedback = std::fs::read_to_string(&file).unwrap_or_else(|e| {
            eprintln!("Failed to read feedback file '{}': {}", file.display(), e);
            process::exit(2);
        });
    } else {
        std::io::stdin()
            .read_to_string(&mut raw_feedback)
            .unwrap_or_else(|e| {
                eprintln!("Failed to read feedback from stdin: {}", e);
                process::exit(2);
            });
    }
    if raw_feedback.trim().is_empty() {
        eprintln!("No feedback content provided. Pass --file or pipe an ACP_REVIEW_RESPONSE_V1 block into stdin.");
        process::exit(2);
    }
    let persisted = record_review_feedback(&shared_id, &raw_feedback, source_endpoint.as_deref())
        .unwrap_or_else(|e| {
            eprintln!(
                "Failed to record review feedback for shared room '{}': {}",
                shared_id, e
            );
            process::exit(2);
        });
    println!("Request ID: {}", persisted.feedback.request_id);
    println!("Source endpoint: {}", persisted.feedback.source_endpoint);
    println!(
        "Will send to driver: {}",
        persisted.driver_envelope.is_some()
    );
    if let Some(target_pane_id) = persisted.target_pane_id.as_deref() {
        println!("Target pane: {}", target_pane_id);
    }
    println!("{}", persisted.text);
    println!("Saved feedback JSON: {}", persisted.json_path.display());
    println!("Saved feedback text: {}", persisted.text_path.display());
    if let Some(ref driver_envelope_path) = persisted.driver_envelope_path {
        println!("Saved driver envelope: {}", driver_envelope_path.display());
    }
    dispatch_persisted_feedback_to_driver(&shared_id, &persisted, true, true);
}

fn refresh_room_state_bar(shared_id: &str) {
    if !room_session_is_active(shared_id) {
        return;
    }
    if let Err(e) = send_acp_room_state_to_session(shared_id, shared_id) {
        let _ = append_stream_event(
            ViewStreamKind::Events,
            shared_id,
            "acp_bar_update_failed",
            None,
            format!("Failed to update ACP bar in '{}'", shared_id),
            json!({
                "error": e,
            }),
        );
    }
}

fn auto_enable_review_automation(shared_id: &str) -> Result<(), String> {
    let mut state = load_review_automation_state(shared_id);
    if state.auto_review_enabled {
        if state.worker_started_at.is_none() {
            state.worker_started_at = Some(timestamp_now());
            let _ = save_review_automation_state(shared_id, state);
        }
        return spawn_review_automation_worker_process(shared_id);
    }
    state.auto_review_enabled = true;
    state.auto_send_enabled = true;
    state.worker_started_at = Some(timestamp_now());
    let _ = save_review_automation_state(shared_id, state);
    spawn_review_automation_worker_process(shared_id)
}

fn spawn_review_automation_worker_process(shared_id: &str) -> Result<(), String> {
    let lock_path = review_automation_worker_lock_path(shared_id);
    if lock_path.exists() {
        if let Some(pid) = read_review_worker_lock_pid(&lock_path) {
            if process_id_is_alive(pid) {
                return Ok(());
            }
        }
        let _ = fs::remove_file(&lock_path);
    }
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    process::Command::new(current_exe)
        .arg("--session")
        .arg(shared_id)
        .arg("review")
        .arg("auto-worker")
        .arg(shared_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn read_review_worker_lock_pid(lock_path: &Path) -> Option<u32> {
    let raw = fs::read_to_string(lock_path).ok()?;
    raw.lines().find_map(|line| {
        line.trim()
            .strip_prefix("pid=")
            .and_then(|value| value.parse::<u32>().ok())
    })
}

fn process_id_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn try_acquire_review_worker_lock(shared_id: &str) -> Result<bool, String> {
    let lock_path = review_automation_worker_lock_path(shared_id);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let open_lock = || {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&lock_path)
    };
    match open_lock() {
        Ok(mut file) => {
            let _ = writeln!(file, "pid={}", std::process::id());
            Ok(true)
        },
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if let Some(pid) = read_review_worker_lock_pid(&lock_path) {
                if process_id_is_alive(pid) {
                    return Ok(false);
                }
            }
            let _ = fs::remove_file(&lock_path);
            match open_lock() {
                Ok(mut file) => {
                    let _ = writeln!(file, "pid={}", std::process::id());
                    Ok(true)
                },
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
                Err(e) => Err(e.to_string()),
            }
        },
        Err(e) => Err(e.to_string()),
    }
}

fn review_worker_is_running(shared_id: &str) -> bool {
    let lock_path = review_automation_worker_lock_path(shared_id);
    if !lock_path.exists() {
        return false;
    }
    let Some(pid) = read_review_worker_lock_pid(&lock_path) else {
        return false;
    };
    process_id_is_alive(pid)
}

fn release_review_worker_lock(shared_id: &str) {
    let _ = fs::remove_file(review_automation_worker_lock_path(shared_id));
}

fn resolve_primary_driver_endpoint(shared_id: &str) -> Result<Option<EndpointMetadata>, String> {
    let endpoints = list_room_endpoints(shared_id).map_err(|e| e.to_string())?;
    Ok(endpoints.into_iter().rev().find(|endpoint| {
        endpoint.role == "driver"
            && terminal_provider_command(&endpoint.provider).is_some()
            && endpoint.bound_pane_id.is_some()
    }))
}

fn ensure_auto_reviewer_running(
    shared_id: &str,
    driver_endpoint: &EndpointMetadata,
) -> Result<bool, String> {
    if !room_session_is_active(shared_id) {
        return Ok(false);
    }
    let reviewer_cwd = PathBuf::from(&driver_endpoint.cwd);
    let reviewer_endpoint = register_participant(shared_id, AgentPersona::Reviewer, &reviewer_cwd)
        .map_err(|e| e.to_string())?
        .endpoint;
    let provider_command = terminal_provider_command(&driver_endpoint.provider).unwrap_or("claude");
    let wrapped_provider_command = provider_wrapper_script(
        shared_id,
        &reviewer_endpoint.endpoint_id,
        &reviewer_endpoint.provider,
        &reviewer_endpoint.role,
        provider_command,
    );
    let pane_id = spawn_provider_pane(
        shared_id,
        &wrapped_provider_command,
        PathBuf::from(&reviewer_endpoint.cwd),
        Some(reviewer_endpoint.label.clone()),
        false,
    )?;
    update_endpoint_pane_binding(shared_id, &reviewer_endpoint.endpoint_id, &pane_id)
        .map_err(|e| e.to_string())?;
    let _ = inject_reviewer_bootstrap_into_pane(shared_id, &pane_id, &reviewer_endpoint);
    let _ = ensure_code_view_pane_in_session(shared_id, shared_id, Path::new(&driver_endpoint.cwd));
    let _ = send_acp_room_state_to_session(shared_id, shared_id);
    let _ = append_stream_event(
        ViewStreamKind::Events,
        shared_id,
        "auto_reviewer_spawned",
        Some(&reviewer_endpoint),
        format!(
            "Auto-started reviewer {} on {}",
            reviewer_endpoint.endpoint_id, pane_id
        ),
        json!({
            "endpoint_id": reviewer_endpoint.endpoint_id,
            "target_pane_id": pane_id,
        }),
    );
    Ok(true)
}

pub(crate) fn start_room_review_automation(shared_id: Option<String>) {
    let shared_id = resolve_shared_room_id(shared_id);
    if !crate::agent_control_plane::room_exists(&shared_id) {
        eprintln!("Room '{}' does not exist.", shared_id);
        process::exit(2);
    }
    let mut state = load_review_automation_state(&shared_id);
    state.auto_review_enabled = true;
    state.auto_send_enabled = true;
    if state.worker_started_at.is_none() {
        state.worker_started_at = Some(timestamp_now());
    }
    let state = save_review_automation_state(&shared_id, state).unwrap_or_else(|e| {
        eprintln!(
            "Failed to update review automation state for room '{}': {}",
            shared_id, e
        );
        process::exit(2);
    });
    spawn_review_automation_worker_process(&shared_id).unwrap_or_else(|e| {
        eprintln!(
            "Failed to start review automation worker for room '{}': {}",
            shared_id, e
        );
        process::exit(2);
    });
    refresh_room_state_bar(&shared_id);
    println!("Review automation enabled for room '{}'.", shared_id);
    println!(
        "Auto-send: {}",
        if state.auto_send_enabled { "on" } else { "off" }
    );
}

pub(crate) fn stop_room_review_automation(shared_id: Option<String>) {
    let shared_id = resolve_shared_room_id(shared_id);
    if !crate::agent_control_plane::room_exists(&shared_id) {
        eprintln!("Room '{}' does not exist.", shared_id);
        process::exit(2);
    }
    let mut state = load_review_automation_state(&shared_id);
    state.auto_review_enabled = false;
    let state = save_review_automation_state(&shared_id, state).unwrap_or_else(|e| {
        eprintln!(
            "Failed to update review automation state for room '{}': {}",
            shared_id, e
        );
        process::exit(2);
    });
    refresh_room_state_bar(&shared_id);
    println!("Review automation disabled for room '{}'.", shared_id);
    println!(
        "Auto-send remains {}.",
        if state.auto_send_enabled { "on" } else { "off" }
    );
}

pub(crate) fn set_room_review_auto_send(shared_id: Option<String>, enabled: bool) {
    let shared_id = resolve_shared_room_id(shared_id);
    if !crate::agent_control_plane::room_exists(&shared_id) {
        eprintln!("Room '{}' does not exist.", shared_id);
        process::exit(2);
    }
    let mut state = load_review_automation_state(&shared_id);
    state.auto_send_enabled = enabled;
    save_review_automation_state(&shared_id, state).unwrap_or_else(|e| {
        eprintln!(
            "Failed to update review automation state for room '{}': {}",
            shared_id, e
        );
        process::exit(2);
    });
    refresh_room_state_bar(&shared_id);
    println!(
        "Review auto-send for room '{}' is now {}.",
        shared_id,
        if enabled { "on" } else { "off" }
    );
}

pub(crate) fn print_room_review_automation_status(shared_id: Option<String>) {
    let shared_id = resolve_shared_room_id(shared_id);
    if !crate::agent_control_plane::room_exists(&shared_id) {
        eprintln!("Room '{}' does not exist.", shared_id);
        process::exit(2);
    }
    let state = load_review_automation_state(&shared_id);
    let worker_running = review_worker_is_running(&shared_id);
    println!("Room: {}", shared_id);
    println!(
        "Auto review: {}",
        if state.auto_review_enabled {
            "on"
        } else {
            "off"
        }
    );
    println!(
        "Auto send: {}",
        if state.auto_send_enabled { "on" } else { "off" }
    );
    println!(
        "Worker: {}",
        if worker_running { "running" } else { "stopped" }
    );
    if let Ok(metadata) = room_metadata(&shared_id) {
        println!(
            "Workspace reviewer target: {}",
            metadata.reviewer_target_count.max(1)
        );
        println!(
            "Workspace reviewer prompt: {}",
            if metadata.reviewer_prompt_override.is_some() {
                "custom"
            } else {
                "default"
            }
        );
        println!(
            "Workspace provider args: {}",
            metadata
                .driver_provider_args
                .as_deref()
                .unwrap_or("<none>")
        );
    }
    if let Some(last_request_id) = state.last_request_id {
        println!("Last request: {}", last_request_id);
    }
    if let Some(active_request_id) = state.active_request_id {
        println!("Pending request: {}", active_request_id);
    }
    if let Some(active_request_started_at) = state.active_request_started_at {
        println!("Pending since: {}", active_request_started_at);
    }
    if let Some(updated_at) = Some(state.updated_at) {
        println!("Updated at: {}", updated_at);
    }
}

pub(crate) fn run_room_review_automation_worker(shared_id: Option<String>) {
    let shared_id = resolve_shared_room_id(shared_id);
    if !crate::agent_control_plane::room_exists(&shared_id) {
        return;
    }
    let acquired_lock = try_acquire_review_worker_lock(&shared_id).unwrap_or(false);
    if !acquired_lock {
        return;
    }
    let mut state = load_review_automation_state(&shared_id);
    state.worker_started_at = Some(timestamp_now());
    let _ = save_review_automation_state(&shared_id, state.clone());
    loop {
        state = load_review_automation_state(&shared_id);
        if !state.auto_review_enabled {
            break;
        }
        if let Some(active_request_id) = state.active_request_id.clone() {
            let timed_out = state
                .active_request_started_at
                .as_deref()
                .map(review_request_timed_out)
                .unwrap_or(false);
            if timed_out {
                let _ = append_stream_event(
                    ViewStreamKind::Events,
                    &shared_id,
                    "review_request_timed_out",
                    None,
                    format!(
                        "Timed out waiting for reviewer response for {}",
                        active_request_id
                    ),
                    json!({
                        "request_id": active_request_id,
                        "timeout_secs": REVIEW_REQUEST_TIMEOUT_SECS,
                    }),
                );
                state.active_request_id = None;
                state.active_request_started_at = None;
                let _ = save_review_automation_state(&shared_id, state.clone());
            }
        }
        if room_session_is_active(&shared_id) {
            let _ = backfill_driver_pane_binding_from_live_room(&shared_id, &shared_id);
        }
        let driver_endpoint = match resolve_primary_driver_endpoint(&shared_id) {
            Ok(driver_endpoint) => driver_endpoint,
            Err(_) => {
                thread::sleep(Duration::from_millis(REVIEW_AUTOMATION_POLL_MS));
                continue;
            },
        };
        let Some(driver_endpoint) = driver_endpoint else {
            thread::sleep(Duration::from_millis(REVIEW_AUTOMATION_POLL_MS));
            continue;
        };
        let Some(driver_pane_id) = driver_endpoint.bound_pane_id.clone() else {
            thread::sleep(Duration::from_millis(REVIEW_AUTOMATION_POLL_MS));
            continue;
        };
        let reviewer_target_count = room_metadata(&shared_id)
            .map(|metadata| metadata.reviewer_target_count.max(1))
            .unwrap_or(1);
        let mut reviewer_targets = match resolve_live_bound_reviewer_panes(&shared_id, &shared_id)
        {
            Ok(reviewer_targets) => reviewer_targets,
            Err(_) => {
                thread::sleep(Duration::from_millis(REVIEW_AUTOMATION_POLL_MS));
                continue;
            },
        };
        reviewer_targets = if reviewer_targets.len() < reviewer_target_count {
            let missing = reviewer_target_count.saturating_sub(reviewer_targets.len());
            for _ in 0..missing {
                let _ = ensure_auto_reviewer_running(&shared_id, &driver_endpoint);
            }
            resolve_live_bound_reviewer_panes(&shared_id, &shared_id).unwrap_or_default()
        } else {
            reviewer_targets
        };
        if reviewer_targets.is_empty() {
            thread::sleep(Duration::from_millis(REVIEW_AUTOMATION_POLL_MS));
            continue;
        }

        if let Ok(Some(turn_state)) = driver_turn_state(
            &shared_id,
            &driver_pane_id,
            &driver_endpoint.endpoint_id,
        ) {
            let idle_for = SystemTime::now()
                .duration_since(turn_state.latest_event_at)
                .unwrap_or_default();
            if idle_for >= Duration::from_secs(REVIEW_AUTOMATION_IDLE_SECS)
                && state.active_request_id.is_none()
                && state.last_driver_fingerprint.as_deref() != Some(turn_state.fingerprint.as_str())
            {
                if let Ok(persisted) = prepare_review_request(&shared_id) {
                    dispatch_review_request_to_reviewers(&shared_id, &persisted, false);
                    state.last_driver_fingerprint = Some(turn_state.fingerprint.clone());
                    state.last_request_id = Some(persisted.request.request_id.clone());
                    state.active_request_id = Some(persisted.request.request_id.clone());
                    state.active_request_started_at = Some(timestamp_now());
                    let _ = save_review_automation_state(&shared_id, state.clone());
                }
            }
        }

        let reviewer_pane_ids: HashSet<String> = reviewer_targets
            .iter()
            .map(|(_, pane_id)| pane_id.clone())
            .collect();
        let tracked_request_id = state.active_request_id.clone();
        if let Some(tracked_request_id) = tracked_request_id {
            if let Ok(Some((block, source_pane_id, snapshot_time))) =
                latest_reviewer_response_block(&shared_id, &reviewer_pane_ids)
            {
                if !snapshot_is_fresh_for_request(
                    &snapshot_time,
                    state.active_request_started_at.as_deref(),
                ) {
                    thread::sleep(Duration::from_millis(REVIEW_AUTOMATION_POLL_MS));
                    continue;
                }
                let parsed_request_id = parse_review_response_request_id(&block);
                let request_matches = parsed_request_id
                    .as_deref()
                    .map(|request_id| {
                        request_id == tracked_request_id.as_str()
                            || (is_placeholder_request_id(request_id)
                                && !reviewer_response_is_unfilled_template(&block))
                    })
                    .unwrap_or(false);
                if request_matches {
                    let normalized_block = parsed_request_id
                        .as_deref()
                        .filter(|request_id| is_placeholder_request_id(request_id))
                        .map(|_| normalize_review_response_request_id(&block, &tracked_request_id))
                        .unwrap_or_else(|| block.clone());
                    let stop_requested = review_response_requests_loop_end(&normalized_block);
                    let block_fingerprint = stable_fingerprint(&block);
                    if state.last_response_fingerprint.as_deref()
                        != Some(block_fingerprint.as_str())
                    {
                        let source_endpoint_override = reviewer_targets
                            .iter()
                            .find(|(_, pane_id)| pane_id == &source_pane_id)
                            .map(|(endpoint, _)| endpoint.endpoint_id.as_str());
                        if let Ok(persisted_feedback) =
                            record_review_feedback(
                                &shared_id,
                                &normalized_block,
                                source_endpoint_override,
                            )
                        {
                            dispatch_persisted_feedback_to_driver(
                                &shared_id,
                                &persisted_feedback,
                                state.auto_send_enabled,
                                false,
                            );
                            state.last_response_fingerprint = Some(block_fingerprint);
                            if state.active_request_id.as_deref()
                                == Some(persisted_feedback.feedback.request_id.as_str())
                            {
                                state.active_request_id = None;
                                state.active_request_started_at = None;
                            }
                            if stop_requested {
                                state.auto_review_enabled = false;
                                let _ = append_stream_event(
                                    ViewStreamKind::Events,
                                    &shared_id,
                                    "review_loop_converged",
                                    None,
                                    format!(
                                        "Reviewer requested convergence on {}",
                                        persisted_feedback.feedback.request_id
                                    ),
                                    json!({
                                        "request_id": persisted_feedback.feedback.request_id,
                                        "loop_control": REVIEW_LOOP_CONTROL_END,
                                    }),
                                );
                            }
                            let _ = save_review_automation_state(&shared_id, state.clone());
                            if stop_requested {
                                break;
                            }
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(REVIEW_AUTOMATION_POLL_MS));
    }
    release_review_worker_lock(&shared_id);
}

#[cfg(feature = "web_server_capability")]
pub(crate) fn stop_web_server() -> Result<(), String> {
    shutdown_all_webserver_instances().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(feature = "web_server_capability"))]
pub(crate) fn stop_web_server() -> Result<(), String> {
    log::error!(
        "This version of Zellij was compiled without web server support, cannot stop web server!"
    );
    eprintln!(
        "This version of Zellij was compiled without web server support, cannot stop web server!"
    );
    std::process::exit(2);
}

#[cfg(feature = "web_server_capability")]
pub(crate) fn create_auth_token(name: Option<String>, read_only: bool) -> Result<String, String> {
    // returns the token and it's name
    create_token(name, read_only)
        .map(|(token, token_name)| {
            let access_type = if read_only { " (read-only)" } else { "" };
            format!("{}: {}{}", token_name, token, access_type)
        })
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "web_server_capability"))]
pub(crate) fn create_auth_token(_name: Option<String>, _read_only: bool) -> Result<String, String> {
    log::error!(
        "This version of Zellij was compiled without web server support, cannot create auth token!"
    );
    eprintln!(
        "This version of Zellij was compiled without web server support, cannot create auth token!"
    );
    std::process::exit(2);
}

#[cfg(feature = "web_server_capability")]
pub(crate) fn revoke_auth_token(token_name: &str) -> Result<bool, String> {
    revoke_token(token_name).map_err(|e| e.to_string())
}

#[cfg(not(feature = "web_server_capability"))]
pub(crate) fn revoke_auth_token(_token_name: &str) -> Result<bool, String> {
    log::error!(
        "This version of Zellij was compiled without web server support, cannot revoke auth token!"
    );
    eprintln!(
        "This version of Zellij was compiled without web server support, cannot revoke auth token!"
    );
    std::process::exit(2);
}

#[cfg(feature = "web_server_capability")]
pub(crate) fn revoke_all_auth_tokens() -> Result<usize, String> {
    // returns the revoked count
    revoke_all_tokens().map_err(|e| e.to_string())
}

#[cfg(not(feature = "web_server_capability"))]
pub(crate) fn revoke_all_auth_tokens() -> Result<usize, String> {
    log::error!(
        "This version of Zellij was compiled without web server support, cannot revoke all tokens!"
    );
    eprintln!(
        "This version of Zellij was compiled without web server support, cannot revoke all tokens!"
    );
    std::process::exit(2);
}

#[cfg(feature = "web_server_capability")]
pub(crate) fn list_auth_tokens() -> Result<Vec<String>, String> {
    // returns the token list line by line
    list_tokens()
        .map(|tokens| {
            let mut res = vec![];
            for t in tokens {
                let access_type = if t.read_only { " [READ-ONLY]" } else { "" };
                res.push(format!(
                    "{}: created at {}{}",
                    t.name, t.created_at, access_type
                ))
            }
            res
        })
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "web_server_capability"))]
pub(crate) fn list_auth_tokens() -> Result<Vec<String>, String> {
    log::error!(
        "This version of Zellij was compiled without web server support, cannot list tokens!"
    );
    eprintln!(
        "This version of Zellij was compiled without web server support, cannot list tokens!"
    );
    std::process::exit(2);
}

/// Default timeout for web server status check (in seconds)
pub const DEFAULT_WEB_SERVER_STATUS_TIMEOUT_SECS: u64 = 30;

#[cfg(feature = "web_server_capability")]
pub(crate) fn web_server_status(
    web_server_base_url: &str,
    timeout_secs: Option<u64>,
) -> Result<String, String> {
    let timeout =
        Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_WEB_SERVER_STATUS_TIMEOUT_SECS));
    let http_client = HttpClient::builder()
        .timeout(timeout)
        .redirect_policy(RedirectPolicy::Follow)
        .build()
        .map_err(|e| e.to_string())?;
    let request = Request::get(format!("{}/info/version", web_server_base_url,));
    let req = request.body(()).map_err(|e| e.to_string())?;
    let mut res = http_client.send(req).map_err(|e| e.to_string())?;
    let status_code = res.status();
    if status_code == 200 {
        let body = res.bytes().map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&body).to_string())
    } else {
        Err(format!(
            "Failed to stop web server, got status code: {}",
            status_code
        ))
    }
}

#[cfg(not(feature = "web_server_capability"))]
pub(crate) fn web_server_status(
    _web_server_base_url: &str,
    _timeout_secs: Option<u64>,
) -> Result<String, String> {
    log::error!(
        "This version of Zellij was compiled without web server support, cannot get web server status!"
    );
    eprintln!(
        "This version of Zellij was compiled without web server support, cannot get web server status!"
    );
    std::process::exit(2);
}

fn find_indexed_session(
    sessions: Vec<String>,
    config_options: Options,
    index: usize,
    create: bool,
) -> ClientInfo {
    match sessions.get(index) {
        Some(session) => ClientInfo::Attach(session.clone(), config_options),
        None if create => create_new_client(),
        None => {
            println!(
                "No session indexed by {} found. The following sessions are active:",
                index
            );
            print_sessions_with_index(sessions);
            process::exit(1);
        },
    }
}

/// Client entrypoint for all [`zellij_utils::cli::CliAction`]
///
/// Checks session to send the action to and attaches with client
pub(crate) fn send_action_to_session(
    cli_action: zellij_utils::cli::CliAction,
    requested_session_name: Option<String>,
    config: Option<Config>,
) {
    match get_active_session() {
        ActiveSession::None => {
            eprintln!("There is no active session!");
            std::process::exit(1);
        },
        ActiveSession::One(session_name) => {
            if let Some(requested_session_name) = requested_session_name {
                if requested_session_name != session_name {
                    eprintln!(
                        "Session '{}' not found. The following sessions are active:",
                        requested_session_name
                    );
                    eprintln!("{}", session_name);
                    std::process::exit(1);
                }
            }
            attach_with_cli_client(cli_action, &session_name, config);
        },
        ActiveSession::Many => {
            let existing_sessions: Vec<String> = get_sessions()
                .unwrap_or_default()
                .iter()
                .map(|s| s.0.clone())
                .collect();
            if let Some(session_name) = requested_session_name {
                if existing_sessions.contains(&session_name) {
                    attach_with_cli_client(cli_action, &session_name, config);
                } else {
                    eprintln!(
                        "Session '{}' not found. The following sessions are active:",
                        session_name
                    );
                    list_sessions(false, false, true);
                    std::process::exit(1);
                }
            } else if let Ok(session_name) = envs::get_session_name() {
                attach_with_cli_client(cli_action, &session_name, config);
            } else {
                eprintln!("Please specify the session name to send actions to. The following sessions are active:");
                list_sessions(false, false, true);
                std::process::exit(1);
            }
        },
    };
}
pub(crate) fn subscribe_to_session(
    subscribe_cli: zellij_utils::cli::SubscribeCli,
    requested_session_name: Option<String>,
    _config: Option<Config>,
) {
    let session_name = match get_active_session() {
        ActiveSession::None => {
            eprintln!("There is no active session!");
            std::process::exit(1);
        },
        ActiveSession::One(session_name) => {
            if let Some(ref requested) = requested_session_name {
                if *requested != session_name {
                    eprintln!(
                        "Session '{}' not found. The following sessions are active:",
                        requested
                    );
                    eprintln!("{}", session_name);
                    std::process::exit(1);
                }
            }
            session_name
        },
        ActiveSession::Many => {
            let existing_sessions: Vec<String> = get_sessions()
                .unwrap_or_default()
                .iter()
                .map(|s| s.0.clone())
                .collect();
            if let Some(session_name) = requested_session_name {
                if existing_sessions.contains(&session_name) {
                    session_name
                } else {
                    eprintln!(
                        "Session '{}' not found. The following sessions are active:",
                        session_name
                    );
                    list_sessions(false, false, true);
                    std::process::exit(1);
                }
            } else if let Ok(session_name) = envs::get_session_name() {
                session_name
            } else {
                eprintln!("Please specify the session name to subscribe to. The following sessions are active:");
                list_sessions(false, false, true);
                std::process::exit(1);
            }
        },
    };
    let os_input = get_os_input(zellij_client::os_input_output::get_cli_client_os_input);
    zellij_client::cli_client::start_subscribe_client(
        Box::new(os_input),
        &session_name,
        subscribe_cli,
    );
}

pub(crate) fn convert_old_config_file(old_config_file: PathBuf) {
    match File::open(&old_config_file) {
        Ok(mut handle) => {
            let mut raw_config_file = String::new();
            let _ = handle.read_to_string(&mut raw_config_file);
            match config_yaml_to_config_kdl(&raw_config_file, false) {
                Ok(kdl_config) => {
                    println!("{}", kdl_config);
                    process::exit(0);
                },
                Err(e) => {
                    eprintln!("Failed to convert config: {}", e);
                    process::exit(1);
                },
            }
        },
        Err(e) => {
            eprintln!("Failed to open file: {}", e);
            process::exit(1);
        },
    }
}

pub(crate) fn convert_old_layout_file(old_layout_file: PathBuf) {
    match File::open(&old_layout_file) {
        Ok(mut handle) => {
            let mut raw_layout_file = String::new();
            let _ = handle.read_to_string(&mut raw_layout_file);
            match layout_yaml_to_layout_kdl(&raw_layout_file) {
                Ok(kdl_layout) => {
                    println!("{}", kdl_layout);
                    process::exit(0);
                },
                Err(e) => {
                    eprintln!("Failed to convert layout: {}", e);
                    process::exit(1);
                },
            }
        },
        Err(e) => {
            eprintln!("Failed to open file: {}", e);
            process::exit(1);
        },
    }
}

pub(crate) fn convert_old_theme_file(old_theme_file: PathBuf) {
    match File::open(&old_theme_file) {
        Ok(mut handle) => {
            let mut raw_config_file = String::new();
            let _ = handle.read_to_string(&mut raw_config_file);
            match config_yaml_to_config_kdl(&raw_config_file, true) {
                Ok(kdl_config) => {
                    println!("{}", kdl_config);
                    process::exit(0);
                },
                Err(e) => {
                    eprintln!("Failed to convert config: {}", e);
                    process::exit(1);
                },
            }
        },
        Err(e) => {
            eprintln!("Failed to open file: {}", e);
            process::exit(1);
        },
    }
}

fn attach_with_cli_client(
    cli_action: zellij_utils::cli::CliAction,
    session_name: &str,
    config: Option<Config>,
) {
    match dispatch_cli_action_to_session(cli_action, session_name, config) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("{}", e);
            log::error!("Error sending action: {}", e);
            std::process::exit(2);
        },
    }
}

fn dispatch_cli_action_to_session(
    cli_action: zellij_utils::cli::CliAction,
    session_name: &str,
    config: Option<Config>,
) -> Result<(), String> {
    let os_input = get_os_input(zellij_client::os_input_output::get_cli_client_os_input);
    let get_current_dir = || std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match Action::actions_from_cli(cli_action, Box::new(get_current_dir), config) {
        Ok(actions) => {
            zellij_client::cli_client::start_cli_client(Box::new(os_input), session_name, actions);
            Ok(())
        },
        Err(e) => Err(e.to_string()),
    }
}

fn attach_with_session_index(config_options: Options, index: usize, create: bool) -> ClientInfo {
    // Ignore the session_name when `--index` is provided
    match get_sessions_sorted_by_mtime() {
        Ok(sessions) if sessions.is_empty() => {
            if create {
                create_new_client()
            } else {
                eprintln!("No active zellij sessions found.");
                process::exit(1);
            }
        },
        Ok(sessions) => find_indexed_session(sessions, config_options, index, create),
        Err(e) => {
            eprintln!("Error occurred: {:?}", e);
            process::exit(1);
        },
    }
}

fn attach_with_session_name(
    session_name: Option<String>,
    config_options: Options,
    create: bool,
) -> ClientInfo {
    match &session_name {
        Some(session) if create => {
            if session_exists(session).unwrap() {
                ClientInfo::Attach(session_name.unwrap(), config_options)
            } else {
                ClientInfo::New(session_name.unwrap(), None, None)
            }
        },
        Some(prefix) => match match_session_name(prefix).unwrap() {
            SessionNameMatch::UniquePrefix(s) | SessionNameMatch::Exact(s) => {
                ClientInfo::Attach(s, config_options)
            },
            SessionNameMatch::AmbiguousPrefix(sessions) => {
                println!(
                    "Ambiguous selection: multiple sessions names start with '{}':",
                    prefix
                );
                print_sessions(
                    sessions
                        .iter()
                        .map(|s| (s.clone(), Duration::default(), false))
                        .collect(),
                    false,
                    false,
                    true,
                );
                process::exit(1);
            },
            SessionNameMatch::None => {
                eprintln!("No session with the name '{}' found!", prefix);
                process::exit(1);
            },
        },
        None => match get_active_session() {
            ActiveSession::None if create => create_new_client(),
            ActiveSession::None => {
                eprintln!("No active zellij sessions found.");
                process::exit(1);
            },
            ActiveSession::One(session_name) => ClientInfo::Attach(session_name, config_options),
            ActiveSession::Many => {
                println!("Please specify the session to attach to, either by using the full name or a unique prefix.\nThe following sessions are active:");
                list_sessions(false, false, true);
                process::exit(1);
            },
        },
    }
}

pub(crate) fn start_client(opts: CliArgs) {
    // look for old YAML config/layout/theme files and convert them to KDL
    convert_old_yaml_files(&opts);
    let (
        config,
        client_layout_info,
        config_options,
        mut config_without_layout,
        mut config_options_without_layout,
    ) = match Setup::from_cli_args(&opts) {
        Ok(results) => results,
        Err(e) => {
            if let ConfigError::KdlError(error) = e {
                let report: Report = error.into();
                eprintln!("{:?}", report);
            } else {
                eprintln!("{}", e);
            }
            process::exit(1);
        },
    };

    let mut reconnect_to_session: Option<ConnectToSession> = None;
    let os_input = get_os_input(get_client_os_input);
    loop {
        let os_input = os_input.clone();
        let mut config = config.clone();
        let mut config_options = config_options.clone();
        let mut opts = opts.clone();
        let mut is_a_reconnect = false;
        let mut should_create_detached = false;
        let mut layout_info = client_layout_info.clone();
        let mut new_session_cwd = None;

        if let Some(reconnect_to_session) = &reconnect_to_session {
            // this is integration code to make session reconnects work with this existing,
            // untested and pretty involved function
            //
            // ideally, we should write tests for this whole function and refctor it
            reload_config_from_disk(
                &mut config_without_layout,
                &mut config_options_without_layout,
                &opts,
            );
            if reconnect_to_session.name.is_some() {
                opts.command = Some(Command::Sessions(Sessions::Attach {
                    session_name: reconnect_to_session.name.clone(),
                    create: true,
                    create_background: false,
                    force_run_commands: false,
                    index: None,
                    options: None,
                    token: None,
                    remember: false,
                    forget: false,
                    ca_cert: None,
                    insecure: false,
                }));
            } else {
                opts.command = None;
                opts.session = None;
                config_options.attach_to_session = None;
            }

            if let Some(reconnect_layout) = &reconnect_to_session.layout {
                layout_info = Some(reconnect_layout.clone());
            }
            if let Some(cwd) = &reconnect_to_session.cwd {
                new_session_cwd = Some(cwd.clone());
            }
            config = config_without_layout.clone();
            config_options = config_options_without_layout.clone();
            is_a_reconnect = true;
        }

        let start_client_plan = |session_name: std::string::String| {
            assert_session_ne(&session_name);
        };

        if let Some(Command::Sessions(Sessions::Attach {
            session_name,
            create,
            create_background,
            force_run_commands,
            index,
            options,
            token,
            remember,
            forget,
            ca_cert,
            insecure,
        })) = opts.command.clone()
        {
            if let Some(remote_session_url) = session_name.as_ref().and_then(|s| {
                if s.starts_with("http://") || s.starts_with("https://") {
                    Some(s)
                } else {
                    None
                }
            }) {
                if !cfg!(feature = "web_server_capability") {
                    eprintln!("This version of Zellij was compiled without web/remote-attach capabilities.");
                    std::process::exit(2);
                }

                if options.is_some() || create || create_background || force_run_commands {
                    eprintln!("Cannot attach to remote session with options.");
                    std::process::exit(2);
                }

                #[cfg(feature = "web_server_capability")]
                if let Err(e) = zellij_client::start_remote_client(
                    Box::new(os_input.clone()),
                    remote_session_url,
                    token,
                    remember,
                    forget,
                    ca_cert,
                    insecure,
                    config_options.client_async_worker_tasks,
                ) {
                    eprintln!("{}", e);
                    std::process::exit(2);
                }
            } else {
                let config_options = match options.as_deref() {
                    Some(SessionCommand::Options(o)) => {
                        config_options.merge_from_cli(o.to_owned().into())
                    },
                    None => config_options,
                };
                should_create_detached = create_background;

                let mut client = if let Some(idx) = index {
                    attach_with_session_index(
                        config_options.clone(),
                        idx,
                        create || should_create_detached,
                    )
                } else {
                    let session_exists = session_name
                        .as_ref()
                        .and_then(|s| session_exists(&s).ok())
                        .unwrap_or(false);
                    let resurrection_layout =
                        session_name
                            .as_ref()
                            .and_then(|s| match resurrection_layout(&s) {
                                Ok(layout) => layout,
                                Err(e) => {
                                    eprintln!("{}", e);
                                    process::exit(2);
                                },
                            });
                    if (create || should_create_detached)
                        && !session_exists
                        && resurrection_layout.is_none()
                    {
                        session_name.clone().map(start_client_plan);
                    }
                    match (session_name.as_ref(), resurrection_layout) {
                        (Some(session_name), Some(mut resurrection_layout)) if !session_exists => {
                            if force_run_commands {
                                resurrection_layout.recursively_add_start_suspended(Some(false));
                            }
                            ClientInfo::Resurrect(
                                session_name.clone(),
                                session_layout_cache_file_name(session_name.as_ref()),
                                force_run_commands,
                                new_session_cwd.clone(),
                            )
                        },
                        _ => attach_with_session_name(
                            session_name,
                            config_options.clone(),
                            create || should_create_detached,
                        ),
                    }
                };

                if let Ok(val) = std::env::var(envs::SESSION_NAME_ENV_KEY) {
                    if val == *client.get_session_name() {
                        panic!("You are trying to attach to the current session (\"{}\"). This is not supported.", val);
                    }
                }

                if let Some(layout_info) = layout_info {
                    client.set_layout_info(layout_info);
                }

                if let Some(new_session_cwd) = new_session_cwd {
                    client.set_cwd(new_session_cwd);
                }

                let tab_position_to_focus = reconnect_to_session
                    .as_ref()
                    .and_then(|r| r.tab_position.clone());
                let pane_id_to_focus = reconnect_to_session
                    .as_ref()
                    .and_then(|r| r.pane_id.clone());
                reconnect_to_session = start_client_impl(
                    Box::new(os_input),
                    opts,
                    config,
                    config_options,
                    client,
                    tab_position_to_focus,
                    pane_id_to_focus,
                    is_a_reconnect,
                    should_create_detached,
                );
            }
        } else {
            if let Some(session_name) = opts.session.clone() {
                start_client_plan(session_name.clone());
                reconnect_to_session = start_client_impl(
                    Box::new(os_input),
                    opts,
                    config,
                    config_options,
                    ClientInfo::New(session_name, layout_info, new_session_cwd),
                    None,
                    None,
                    is_a_reconnect,
                    should_create_detached,
                );
            } else {
                if let Some(session_name) = config_options.session_name.as_ref() {
                    if let Ok(val) = envs::get_session_name() {
                        // This prevents the same type of recursion as above, only that here we
                        // don't get the command to "attach", but to start a new session instead.
                        // This occurs for example when declaring the session name inside a layout
                        // file and then, from within this session, trying to open a new zellij
                        // session with the same layout. This causes an infinite recursion in the
                        // `zellij_server::terminal_bytes::listen` task, flooding the server and
                        // clients with infinite `Render` requests.
                        if *session_name == val {
                            eprintln!("You are trying to attach to the current session (\"{}\"). Zellij does not support nesting a session in itself.", session_name);
                            process::exit(1);
                        }
                    }
                    match config_options.attach_to_session {
                        Some(true) => {
                            let client = attach_with_session_name(
                                Some(session_name.clone()),
                                config_options.clone(),
                                true,
                            );
                            reconnect_to_session = start_client_impl(
                                Box::new(os_input),
                                opts,
                                config,
                                config_options,
                                client,
                                None,
                                None,
                                is_a_reconnect,
                                should_create_detached,
                            );
                        },
                        _ => {
                            start_client_plan(session_name.clone());
                            reconnect_to_session = start_client_impl(
                                Box::new(os_input),
                                opts,
                                config,
                                config_options.clone(),
                                ClientInfo::New(session_name.clone(), layout_info, new_session_cwd),
                                None,
                                None,
                                is_a_reconnect,
                                should_create_detached,
                            );
                        },
                    }
                    if reconnect_to_session.is_some() {
                        continue;
                    }
                    // after we detach, this happens and so we need to exit before the rest of the
                    // function happens
                    process::exit(0);
                }

                let session_name = generate_unique_session_name_or_exit();
                start_client_plan(session_name.clone());
                reconnect_to_session = start_client_impl(
                    Box::new(os_input),
                    opts,
                    config,
                    config_options,
                    ClientInfo::New(session_name, layout_info, new_session_cwd),
                    None,
                    None,
                    is_a_reconnect,
                    should_create_detached,
                );
            }
        }
        if reconnect_to_session.is_none() {
            break;
        }
    }
}

fn generate_unique_session_name_or_exit() -> String {
    let Some(unique_session_name) = generate_unique_session_name() else {
        eprintln!("Failed to generate a unique session name, giving up");
        process::exit(1);
    };
    unique_session_name
}

pub(crate) fn list_aliases(opts: CliArgs) {
    let (config, _layout, _config_options, _config_without_layout, _config_options_without_layout) =
        match Setup::from_cli_args(&opts) {
            Ok(results) => results,
            Err(e) => {
                if let ConfigError::KdlError(error) = e {
                    let report: Report = error.into();
                    eprintln!("{:?}", report);
                } else {
                    eprintln!("{}", e);
                }
                process::exit(1);
            },
        };
    for alias in config.plugins.list() {
        println!("{}", alias);
    }
    process::exit(0);
}

pub(crate) fn watch_session(session_name: Option<String>, opts: CliArgs) {
    let (config, _, config_options, _, _) = match Setup::from_cli_args(&opts) {
        Ok(results) => results,
        Err(e) => {
            if let ConfigError::KdlError(error) = e {
                let report: Report = error.into();
                eprintln!("{:?}", report);
            } else {
                eprintln!("{}", e);
            }
            process::exit(1);
        },
    };

    // Resolve the session name to watch
    let client_info = match &session_name {
        Some(prefix) => match match_session_name(prefix).unwrap() {
            SessionNameMatch::UniquePrefix(s) | SessionNameMatch::Exact(s) => {
                ClientInfo::Watch(s, config_options.clone())
            },
            SessionNameMatch::AmbiguousPrefix(sessions) => {
                eprintln!(
                    "Ambiguous selection: multiple sessions names start with '{}':",
                    prefix
                );
                print_sessions(
                    sessions
                        .iter()
                        .map(|s| (s.clone(), Duration::default(), false))
                        .collect(),
                    false,
                    false,
                    true,
                );
                process::exit(1);
            },
            SessionNameMatch::None => {
                eprintln!("No session with the name '{}' found!", prefix);
                process::exit(1);
            },
        },
        None => match get_active_session() {
            ActiveSession::None => {
                eprintln!("No active zellij sessions found.");
                process::exit(1);
            },
            ActiveSession::One(name) => ClientInfo::Watch(name, config_options.clone()),
            ActiveSession::Many => {
                eprintln!("Please specify the session name to watch.");
                process::exit(1);
            },
        },
    };

    let mut opts = opts.clone();
    opts.session = Some(client_info.get_session_name().to_string());

    let os_input = get_os_input(get_client_os_input);

    // Start the watcher client
    start_client_impl(
        Box::new(os_input),
        opts,
        config,
        config_options,
        client_info,
        None,  // tab_position_to_focus
        None,  // pane_id_to_focus
        false, // is_a_reconnect
        false, // should_create_detached
    );
}

fn reload_config_from_disk(
    config_without_layout: &mut Config,
    config_options_without_layout: &mut Options,
    opts: &CliArgs,
) {
    match Setup::from_cli_args(&opts) {
        Ok((_, _, _, reloaded_config_without_layout, reloaded_config_options_without_layout)) => {
            *config_without_layout = reloaded_config_without_layout;
            *config_options_without_layout = reloaded_config_options_without_layout;
        },
        Err(e) => {
            log::error!("Failed to reload config: {}", e);
        },
    };
}

pub fn get_config_options_from_cli_args(opts: &CliArgs) -> Result<Options, String> {
    Setup::from_cli_args(&opts)
        .map(|(_, _, config_options, _, _)| config_options)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_endpoint(provider: &str, role: &str, cwd: &str) -> EndpointMetadata {
        EndpointMetadata {
            endpoint_id: format!("{provider}-{role}"),
            shared_id: "room-1".to_owned(),
            session_name: "room-1".to_owned(),
            provider: provider.to_owned(),
            role: role.to_owned(),
            label: format!("{provider} {role}"),
            cwd: cwd.to_owned(),
            pid: 42,
            created_at: "2026-04-04T00:00:00+08:00".to_owned(),
            bound_pane_id: None,
            bound_pane_id_updated_at: None,
            reviewer_bootstrap_prompt: None,
            review_message_template: None,
        }
    }

    #[test]
    fn reviewer_provider_context_prefers_latest_driver_provider_and_cwd() {
        let fallback = Path::new("/fallback");
        let endpoints = vec![
            sample_endpoint("claude", "driver", "/first"),
            sample_endpoint("reviewer", "reviewer", "/ignore"),
            sample_endpoint("codex", "driver", "/second"),
        ];

        let (provider_command, cwd) = reviewer_provider_context(&endpoints, fallback);

        assert_eq!(provider_command, "codex");
        assert_eq!(cwd, PathBuf::from("/second"));
    }

    #[test]
    fn reviewer_provider_context_falls_back_to_claude_and_fallback_cwd() {
        let fallback = Path::new("/fallback");
        let endpoints = vec![sample_endpoint("reviewer", "reviewer", "/ignore")];

        let (provider_command, cwd) = reviewer_provider_context(&endpoints, fallback);

        assert_eq!(provider_command, "claude");
        assert_eq!(cwd, PathBuf::from("/fallback"));
    }

    #[test]
    fn reviewer_bootstrap_message_includes_structured_protocol_and_identity() {
        let mut endpoint = sample_endpoint("reviewer", "reviewer", "/repo");
        endpoint.endpoint_id = "reviewer-abcd1234".to_owned();
        endpoint.reviewer_bootstrap_prompt = Some("bootstrap".to_owned());
        endpoint.review_message_template = Some("[Review from reviewer]".to_owned());

        let bootstrap_message = reviewer_bootstrap_message(&endpoint).unwrap();

        assert!(bootstrap_message.contains("room_id: room-1"));
        assert!(bootstrap_message.contains("source_endpoint: reviewer-abcd1234"));
        assert!(bootstrap_message.contains("[ACP_REVIEW_RESPONSE_V1]"));
        assert!(bootstrap_message.contains("[Review from reviewer]"));
    }

    #[test]
    fn choose_room_id_from_context_prefers_current_session_name() {
        let resolved = choose_room_id_from_context(None, Some("room-123456".to_owned()));

        assert_eq!(resolved, "room-123456");
    }

    #[test]
    fn choose_room_id_from_context_generates_short_numeric_when_no_context_exists() {
        let resolved = choose_room_id_from_context(None, None);

        assert_eq!(resolved.len(), 6);
        assert!(resolved.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn extracts_latest_review_response_block_from_snapshot_text() {
        let text = r#"
[ACP_REVIEW_RESPONSE_V1]
request_id: rr-0001
summary: old
[/ACP_REVIEW_RESPONSE_V1]

noise

[ACP_REVIEW_RESPONSE_V1]
request_id: rr-0002
summary: latest
[/ACP_REVIEW_RESPONSE_V1]
"#;

        let block = extract_latest_response_block_from_text(text).unwrap();

        assert!(block.contains("request_id: rr-0002"));
        assert!(block.contains("summary: latest"));
    }

    #[test]
    fn parses_request_id_from_review_response_block() {
        let block = r#"
[ACP_REVIEW_RESPONSE_V1]
room_id: demo
request_id: rr-0042
summary: ok
[/ACP_REVIEW_RESPONSE_V1]
"#;

        assert_eq!(
            parse_review_response_request_id(block).as_deref(),
            Some("rr-0042")
        );
    }

    #[test]
    fn detects_unfilled_reviewer_template_response() {
        let block = r#"
[ACP_REVIEW_RESPONSE_V1]
room_id: demo
request_id: <request_id>
source_endpoint: reviewer-1
target_role: driver
severity: <low|medium|high>
confidence: <low|medium|high>
should_send: true
summary: <one-line summary>

findings:
- <finding>

actions:
- <action>
[/ACP_REVIEW_RESPONSE_V1]
"#;

        assert!(reviewer_response_is_unfilled_template(block));
    }

    #[test]
    fn does_not_mark_filled_reviewer_response_as_template() {
        let block = r#"
[ACP_REVIEW_RESPONSE_V1]
room_id: demo
request_id: rr-0042
source_endpoint: reviewer-1
target_role: driver
severity: low
confidence: high
should_send: true
summary: Looks good

findings:
- no regression found
[/ACP_REVIEW_RESPONSE_V1]
"#;

        assert!(!reviewer_response_is_unfilled_template(block));
    }

    #[test]
    fn parses_loop_control_and_detects_end_review() {
        let block = r#"
[ACP_REVIEW_RESPONSE_V1]
room_id: demo
request_id: rr-0042
source_endpoint: reviewer-1
target_role: driver
loop_control: END_REVIEW
summary: done
[/ACP_REVIEW_RESPONSE_V1]
"#;

        assert_eq!(
            parse_review_response_loop_control(block).as_deref(),
            Some("END_REVIEW")
        );
        assert!(review_response_requests_loop_end(block));
    }
}
