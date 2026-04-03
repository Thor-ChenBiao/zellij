use crate::agent_control_plane::{
    append_stream_event, follow_stream, list_room_endpoints, prepare_review_request,
    record_review_feedback, register_participant, room_metadata, AgentPersona, EndpointMetadata,
    ViewStreamKind,
};
use dialoguer::Confirm;
use std::net::IpAddr;
use std::{fs::File, io::prelude::*, path::PathBuf, process, time::Duration};

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

fn provider_command_for_persona(persona: AgentPersona) -> Option<&'static str> {
    match persona {
        AgentPersona::Claude => Some("claude"),
        AgentPersona::Codex => Some("codex"),
        AgentPersona::Gemini => Some("gemini"),
        AgentPersona::Reviewer => None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RoomRoleCounts {
    driver_count: usize,
    reviewer_count: usize,
    human_count: usize,
}

fn kdl_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
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
    let counts = count_room_roles(&endpoints);
    let payload = serde_json::to_string(&json!({
        "room_id": shared_id,
        "driver_count": counts.driver_count,
        "reviewer_count": counts.reviewer_count,
        "human_count": counts.human_count,
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
    let shared_id = kdl_string(shared_id);
    let cwd = kdl_string(&cwd.display().to_string());
    let self_provider = kdl_string(&endpoint.provider);
    let self_role = kdl_string(&endpoint.role);
    let acp_bar = format!(
        "        pane size=1 borderless=true {{\n            plugin location=\"zellij:acp-bar\" {{\n                room_id {shared_id}\n                self_provider {self_provider}\n                self_role {self_role}\n                driver_count {}\n                reviewer_count {}\n                human_count {}\n            }}\n        }}",
        role_counts.driver_count, role_counts.reviewer_count, role_counts.human_count
    );
    match provider_command {
        Some(provider_command) => {
            let command = kdl_string(provider_command);
            format!(
                "layout {{\n    tab name={shared_id} focus=true {{\n        pane size=1 borderless=true {{\n            plugin location=\"tab-bar\"\n        }}\n        pane command={command} cwd={cwd} focus=true\n        pane size=1 borderless=true {{\n            plugin location=\"status-bar\"\n        }}\n{acp_bar}\n    }}\n}}"
            )
        },
        None => format!(
            "layout {{\n    tab name={shared_id} focus=true {{\n        pane size=1 borderless=true {{\n            plugin location=\"tab-bar\"\n        }}\n        pane cwd={cwd} focus=true\n        pane size=1 borderless=true {{\n            plugin location=\"status-bar\"\n        }}\n{acp_bar}\n    }}\n}}"
        ),
    }
}

fn spawn_provider_pane(
    session_name: &str,
    provider_command: &str,
    cwd: PathBuf,
    pane_name: Option<String>,
) -> Result<(), String> {
    dispatch_cli_action_to_session(
        CliAction::NewPane {
            direction: None,
            command: vec![provider_command.to_owned()],
            plugin: None,
            cwd: Some(cwd),
            floating: false,
            in_place: false,
            close_replaced_pane: false,
            name: pane_name,
            close_on_exit: false,
            start_suspended: false,
            configuration: None,
            skip_plugin_cache: false,
            x: None,
            y: None,
            width: None,
            height: None,
            pinned: None,
            stacked: false,
            blocking: false,
            block_until_exit_success: false,
            block_until_exit_failure: false,
            block_until_exit: false,
            unblock_condition: None,
            near_current_pane: false,
            borderless: Some(false),
            tab_id: None,
        },
        session_name,
        None,
    )
}

pub(crate) fn start_shared_room(
    mut opts: CliArgs,
    persona: AgentPersona,
    shared_id: Option<String>,
) {
    let shared_id = shared_id.unwrap_or_else(generate_unique_session_name_or_exit);
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let participant = match register_participant(&shared_id, persona, &current_dir) {
        Ok(participant) => participant,
        Err(e) => {
            eprintln!("Failed to register shared room '{}': {}", shared_id, e);
            process::exit(2);
        },
    };
    let _ = append_stream_event(
        ViewStreamKind::Code,
        &shared_id,
        "room_ready",
        Some(&participant.endpoint),
        format!(
            "{} reserved the code stream for session '{}'",
            participant.endpoint.provider, participant.session_name
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
    let room_role_counts = list_room_endpoints(&shared_id)
        .map(|endpoints| count_room_roles(&endpoints))
        .unwrap_or_default();

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

    let provider_command = provider_command_for_persona(persona);
    let session_active = room_session_is_active(&shared_id);

    if participant.room_created || !session_active {
        opts.command = None;
        opts.session = Some(shared_id.clone());
        opts.layout = None;
        opts.new_session_with_layout = None;
        opts.layout_string = Some(room_layout_string(
            &shared_id,
            provider_command,
            &current_dir,
            &participant.endpoint,
            room_role_counts,
        ));
        start_client(opts);
        return;
    }

    if let Some(provider_command) = provider_command {
        if let Err(e) = spawn_provider_pane(
            &shared_id,
            provider_command,
            current_dir.clone(),
            Some(participant.endpoint.label.clone()),
        ) {
            eprintln!(
                "Failed to start {} pane in shared room '{}': {}",
                provider_command, shared_id, e
            );
        }
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

    opts.session = None;
    opts.layout = None;
    opts.new_session_with_layout = None;
    opts.layout_string = None;
    opts.command = Some(attach_create_command(shared_id));
    start_client(opts);
}

pub(crate) fn view_shared_room(stream: ViewStreamKind, shared_id: Option<String>) {
    let shared_id = shared_id.unwrap_or_else(generate_unique_session_name_or_exit);
    let _ = crate::agent_control_plane::ensure_room(&shared_id).unwrap_or_else(|e| {
        eprintln!("Failed to initialize shared room '{}': {}", shared_id, e);
        process::exit(2);
    });
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

pub(crate) fn prepare_room_review_request(shared_id: Option<String>) {
    let shared_id = resolve_shared_room_id(shared_id);
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
    if let Some(driver_envelope_path) = persisted.driver_envelope_path {
        println!("Saved driver envelope: {}", driver_envelope_path.display());
    }
    if let Some(driver_envelope) = persisted.driver_envelope.as_deref() {
        if room_session_is_active(&shared_id) {
            match resolve_driver_delivery_pane(&shared_id, persisted.target_pane_id.as_deref()) {
                Ok(Some(target_pane_id)) => {
                    inject_review_feedback_into_driver(
                        &shared_id,
                        &target_pane_id,
                        driver_envelope,
                        &persisted.feedback.request_id,
                        &persisted.feedback.source_endpoint,
                    )
                    .unwrap_or_else(|e| {
                        eprintln!("Failed to inject driver envelope: {}", e);
                        process::exit(2);
                    });
                    println!("Injected driver envelope into: {}", target_pane_id);
                },
                Ok(None) => {
                    let _ = append_stream_event(
                        ViewStreamKind::Events,
                        &shared_id,
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
                    println!(
                        "Driver pane not resolved. Envelope kept on disk for manual delivery."
                    );
                },
                Err(e) => {
                    let _ = append_stream_event(
                        ViewStreamKind::Events,
                        &shared_id,
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
                    println!("Could not inspect session panes yet. Envelope kept on disk.");
                },
            }
        } else {
            let _ = append_stream_event(
                ViewStreamKind::Events,
                &shared_id,
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
            println!("Room session is not active. Envelope kept on disk.");
        }
    }
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
