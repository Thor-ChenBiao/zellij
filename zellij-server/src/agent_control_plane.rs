use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde_json::json;
use zellij_utils::agent_control_plane::ViewStreamKind;
use zellij_utils::data::{PaneContents, PaneId, PaneRenderReport};

#[derive(Debug, Clone)]
pub(crate) struct StreamEmission {
    pub stream: ViewStreamKind,
    pub kind: String,
    pub message: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AcpSpecialCommand {
    EnsureRoom,
    LaunchProvider { provider: String },
    PrefixWithZellij,
    LaunchViewPane { stream: String, room_id: Option<String> },
}

use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct RoomTelemetry {
    pane_viewport_hashes: HashMap<PaneId, u64>,
    pty_line_buffers: HashMap<u32, String>,
    pane_input_buffers: HashMap<PaneId, String>,
    last_committed_inputs: HashMap<PaneId, String>,
    seen_viewport_tool_calls: HashSet<String>,
}

impl RoomTelemetry {
    pub fn last_committed_input(&self, pane_id: PaneId) -> Option<String> {
        self.last_committed_inputs.get(&pane_id).cloned()
    }

    pub fn ingest_pty_bytes(&mut self, terminal_id: u32, raw_bytes: &[u8]) -> Vec<StreamEmission> {
        let sanitized = sanitize_terminal_output(raw_bytes);
        if sanitized.is_empty() {
            return vec![];
        }

        let buffer = self.pty_line_buffers.entry(terminal_id).or_default();
        buffer.push_str(&sanitized);

        let mut emissions = vec![];
        while let Some(newline_index) = buffer.find('\n') {
            let line: String = buffer.drain(..=newline_index).collect();
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some((tool, value)) = detect_tool_call(trimmed) {
                emissions.push(StreamEmission {
                    stream: ViewStreamKind::Events,
                    kind: "tool_call".to_owned(),
                    message: format!("{}({})", tool, summarize_value(&value)),
                    payload: json!({
                        "terminal_id": terminal_id,
                        "tool": tool,
                        "value": value,
                        "line": trimmed,
                    }),
                });
                // Edit/Write tool calls also emit to code stream with file content
                if matches!(tool.as_str(), "Edit" | "Write" | "MultiEdit") {
                    if let Some(file_path) = extract_file_path_from_tool_value(&value) {
                        let diff_context = read_file_context_for_code_stream(&file_path);
                        emissions.push(StreamEmission {
                            stream: ViewStreamKind::Code,
                            kind: "code_change".to_owned(),
                            message: format!(
                                "━━━ {}: {} ━━━",
                                tool,
                                file_path
                            ),
                            payload: json!({
                                "terminal_id": terminal_id,
                                "tool": tool,
                                "file_path": file_path,
                                "diff_context": diff_context,
                            }),
                        });
                    }
                }
            }
        }
        if buffer.len() > 4096 {
            let keep_from = suffix_start_for_max_bytes(buffer, 1024);
            buffer.drain(..keep_from);
        }
        emissions
    }

    pub fn pane_render_emissions(&mut self, report: &PaneRenderReport) -> Vec<StreamEmission> {
        // Only track viewport hashes for pane_snapshot (used by review request).
        // No viewport scanning — tool calls come from hooks or PTY parsing.
        let Some(pane_map) = report.all_pane_contents.values().next() else {
            return vec![];
        };
        let mut emissions = vec![];
        for (pane_id, pane_contents) in pane_map {
            let viewport_hash = hash_viewport(&pane_contents.viewport);
            let previous_hash = self.pane_viewport_hashes.insert(*pane_id, viewport_hash);
            if previous_hash == Some(viewport_hash) {
                continue;
            }
            // Write pane_snapshot to events (for review request viewport data)
            emissions.push(StreamEmission {
                stream: ViewStreamKind::Events,
                kind: "pane_snapshot".to_owned(),
                message: format!(
                    "Pane {} viewport updated ({} lines)",
                    pane_id,
                    pane_contents.viewport.len()
                ),
                payload: pane_contents_payload(*pane_id, pane_contents),
            });
        }
        emissions
    }

    pub fn ingest_input_bytes(
        &mut self,
        pane_id: PaneId,
        raw_bytes: &[u8],
    ) -> Vec<AcpSpecialCommand> {
        let input = String::from_utf8_lossy(raw_bytes);
        let buffer = self.pane_input_buffers.entry(pane_id).or_default();
        let mut commands = vec![];
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            match ch {
                '\u{1b}' => skip_escape_sequence(&mut chars),
                '\u{8}' | '\u{7f}' => {
                    buffer.pop();
                },
                '\r' | '\n' => {
                    let trimmed = buffer.trim().to_owned();
                    if !trimmed.is_empty() {
                        self.last_committed_inputs.insert(pane_id, trimmed.clone());
                    }
                    if let Some(command) = detect_special_command(&trimmed) {
                        commands.push(command);
                    }
                    buffer.clear();
                },
                _ if ch.is_control() => {},
                _ => buffer.push(ch),
            }
        }

        if buffer.chars().count() > 256 {
            let keep: String = buffer
                .chars()
                .rev()
                .take(128)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            *buffer = keep;
        }
        commands
    }
}

fn pane_contents_payload(pane_id: PaneId, pane_contents: &PaneContents) -> serde_json::Value {
    // Keep a tail snapshot across scrollback + viewport so structured blocks
    // that scrolled above the visible window can still be parsed.
    let max_lines = 240usize;
    let mut snapshot: Vec<String> = Vec::new();
    snapshot.extend(pane_contents.lines_above_viewport.iter().cloned());
    snapshot.extend(pane_contents.viewport.iter().cloned());
    let start = snapshot.len().saturating_sub(max_lines);
    let snapshot_tail: Vec<String> = snapshot.into_iter().skip(start).collect();
    json!({
        "pane_id": pane_id.to_string(),
        "viewport": pane_contents.viewport.clone(),
        "snapshot": snapshot_tail,
        "viewport_line_count": pane_contents.viewport.len(),
        "lines_above_viewport": pane_contents.lines_above_viewport.len(),
        "lines_below_viewport": pane_contents.lines_below_viewport.len(),
    })
}

fn hash_viewport(viewport: &[String]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    viewport.hash(&mut hasher);
    hasher.finish()
}

fn summarize_value(value: &str) -> String {
    let value = value.trim();
    if value.chars().count() <= 120 {
        value.to_owned()
    } else {
        let shortened: String = value.chars().take(117).collect();
        format!("{}...", shortened)
    }
}

fn suffix_start_for_max_bytes(text: &str, max_bytes: usize) -> usize {
    if text.len() <= max_bytes {
        return 0;
    }
    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    start
}

pub(crate) fn sanitize_terminal_output(raw_bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw_bytes);
    let mut cleaned = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            skip_escape_sequence(&mut chars);
            continue;
        }
        if ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t' {
            continue;
        }
        cleaned.push(ch);
    }
    cleaned.replace('\r', "\n")
}

fn skip_escape_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(next) = chars.next() {
        if ('@'..='~').contains(&next) || next == '\u{7}' {
            break;
        }
    }
}

fn detect_tool_call(line: &str) -> Option<(String, String)> {
    const TOOLS: &[&str] = &[
        "Bash",
        "Read",
        "Write",
        "Edit",
        "MultiEdit",
        "Grep",
        "Glob",
        "WebSearch",
        "WebFetch",
        "Task",
        "TodoWrite",
    ];
    for tool in TOOLS {
        let needle = format!("{}(", tool);
        if let Some(start) = line.find(&needle) {
            let rest = &line[start + needle.len()..];
            if let Some(end) = rest.rfind(')') {
                let value = rest[..end].trim();
                if !value.is_empty() {
                    return Some(((*tool).to_owned(), value.to_owned()));
                }
            }
        }
    }
    None
}

fn detect_special_command(line: &str) -> Option<AcpSpecialCommand> {
    let mut parts = line.split_whitespace();
    let first = parts.next()?;
    match first {
        "claude" | "codex" | "gemini" => Some(AcpSpecialCommand::LaunchProvider {
            provider: first.to_owned(),
        }),
        "reviewer" | "review" => Some(AcpSpecialCommand::PrefixWithZellij),
        "view" => {
            // "view code [id]" or "view events [id]"
            let stream = parts.next().unwrap_or("events").to_owned();
            if stream == "code" || stream == "events" {
                let room_id = parts.next().map(|s| s.to_owned());
                Some(AcpSpecialCommand::LaunchViewPane { stream, room_id })
            } else {
                None
            }
        },
        "zellij" => {
            let second = parts.next()?;
            match second {
                "claude" | "codex" | "gemini" | "reviewer" | "review" => {
                    Some(AcpSpecialCommand::EnsureRoom)
                },
                "view" => {
                    let stream = parts.next().unwrap_or("events").to_owned();
                    if stream == "code" || stream == "events" {
                        let room_id = parts.next().map(|s| s.to_owned());
                        Some(AcpSpecialCommand::LaunchViewPane { stream, room_id })
                    } else {
                        None
                    }
                },
                _ => None,
            }
        },
        _ => None,
    }
}

fn extract_file_path_from_tool_value(value: &str) -> Option<String> {
    // Try to extract file_path from tool call value like:
    // file_path="/Users/bill/zellij/src/main.rs", ...
    // or just a bare path like: /Users/bill/zellij/src/main.rs
    for segment in value.split(',') {
        let segment = segment.trim();
        if let Some(rest) = segment
            .strip_prefix("file_path=")
            .or_else(|| segment.strip_prefix("file_path=\""))
        {
            let path = rest.trim_matches('"').trim();
            if !path.is_empty() {
                return Some(path.to_owned());
            }
        }
    }
    // Fallback: if the entire value looks like a file path
    let trimmed = value.trim().trim_matches('"');
    if !trimmed.is_empty() && !trimmed.contains(' ') && (trimmed.contains('/') || trimmed.contains('.')) {
        return Some(trimmed.to_owned());
    }
    None
}

fn read_file_context_for_code_stream(file_path: &str) -> String {
    let path = std::path::Path::new(file_path);
    // Try absolute path first, then resolve relative to common locations
    let resolved = if path.exists() && path.is_file() {
        path.to_path_buf()
    } else if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join(file_path);
        if candidate.exists() && candidate.is_file() {
            candidate
        } else {
            // Try home dir
            if let Ok(home) = std::env::var("HOME") {
                let candidate = std::path::PathBuf::from(home).join(file_path);
                if candidate.exists() && candidate.is_file() {
                    candidate
                } else {
                    return String::new();
                }
            } else {
                return String::new();
            }
        }
    } else {
        return String::new();
    };
    let path = &resolved;
    if !path.exists() || !path.is_file() {
        return String::new();
    }
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let total = lines.len();
            let mut result = Vec::new();
            result.push(format!("@@ {}: {} ({} lines) @@", file_path, path.display(), total));
            let show_lines = if total <= 60 { total } else { 40 };
            for (i, line) in lines.iter().take(show_lines).enumerate() {
                result.push(format!("+{:4} | {}", i + 1, line));
            }
            if total > show_lines {
                result.push(format!("      | ... +{} more lines ...", total - show_lines));
            }
            result.join("\n")
        },
        Err(_) => String::new(),
    }
}

fn detect_wrote_result(line: &str) -> Option<(String, usize)> {
    // Match: "Wrote 187 lines to code-review-test/task_queue.py"
    // or:    "⎿  Wrote 109 lines to code-review-test/cache_manager.py"
    let rest = line
        .find("Wrote ")
        .map(|i| &line[i + "Wrote ".len()..])?;
    let parts: Vec<&str> = rest.splitn(3, ' ').collect();
    if parts.len() >= 3 && parts[1] == "lines" {
        let count = parts[0].parse::<usize>().ok()?;
        let path = parts[2].strip_prefix("to ").unwrap_or(parts[2]).trim();
        if !path.is_empty() && path.contains('/') {
            return Some((path.to_owned(), count));
        }
    }
    None
}

fn extract_file_hint(line: &str) -> Option<String> {
    const FILE_EXTENSIONS: &[&str] = &[
        ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".java", ".kt", ".kts", ".c", ".cc",
        ".cpp", ".h", ".hpp", ".json", ".yaml", ".yml", ".toml", ".md", ".txt", ".sh", ".bash",
        ".zsh", ".html", ".css", ".scss",
    ];

    line.split_whitespace()
        .map(trim_candidate_token)
        .find(|candidate| {
            candidate.contains('/')
                && FILE_EXTENSIONS
                    .iter()
                    .any(|extension| candidate.ends_with(extension))
        })
}

fn trim_candidate_token(token: &str) -> String {
    token
        .trim_matches(|c: char| {
            c == '"' || c == '\'' || c == '(' || c == ')' || c == '[' || c == ']' || c == ','
        })
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tool_calls_and_code_changes_from_terminal_lines() {
        let mut telemetry = RoomTelemetry::default();
        let emissions = telemetry.ingest_pty_bytes(
            7,
            b"\xe2\x8f\xba Write(src/main.rs)\n\xe2\x8f\xba Bash(cargo test -p zellij-server)\n",
        );

        // Write(src/main.rs) -> tool_call + code_change
        // Bash(cargo test ...) -> tool_call
        assert_eq!(emissions.len(), 3);
        assert_eq!(emissions[0].kind, "tool_call");
        assert_eq!(emissions[1].kind, "code_change");
        assert_eq!(emissions[1].stream, ViewStreamKind::Code);
        assert_eq!(emissions[2].kind, "tool_call");
        assert_eq!(emissions[2].stream, ViewStreamKind::Events);
    }

    #[test]
    fn deduplicates_identical_viewport_snapshots() {
        let mut telemetry = RoomTelemetry::default();
        let pane_id = PaneId::Terminal(3);
        let pane_contents = PaneContents {
            lines_above_viewport: vec![],
            lines_below_viewport: vec![],
            viewport: vec!["let value = 1;".to_owned()],
            selected_text: None,
        };
        let mut report = PaneRenderReport::default();
        report
            .all_pane_contents
            .entry(1)
            .or_default()
            .insert(pane_id, pane_contents.clone());

        let first = telemetry.pane_render_emissions(&report);
        let second = telemetry.pane_render_emissions(&report);

        // First render: emits pane_snapshot
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, "pane_snapshot");
        // Second render: same viewport, deduped
        assert!(second.is_empty());
    }

    #[test]
    fn trims_long_multibyte_partial_lines_without_panicking() {
        let mut telemetry = RoomTelemetry::default();
        let input = "你".repeat(2000);

        let emissions = telemetry.ingest_pty_bytes(9, input.as_bytes());
        let buffer = telemetry.pty_line_buffers.get(&9).unwrap();

        assert!(emissions.is_empty());
        assert!(buffer.len() <= 1024);
        assert!(buffer.chars().all(|ch| ch == '你'));
    }

    #[test]
    fn detects_plain_provider_launches_from_user_input() {
        let mut telemetry = RoomTelemetry::default();
        let commands = telemetry.ingest_input_bytes(PaneId::Terminal(1), b"claude --model opus\r");
        assert_eq!(
            commands,
            vec![AcpSpecialCommand::LaunchProvider {
                provider: "claude".to_owned()
            }]
        );
    }

    #[test]
    fn detects_zellij_acp_commands_from_user_input() {
        let mut telemetry = RoomTelemetry::default();
        let commands =
            telemetry.ingest_input_bytes(PaneId::Terminal(1), b"zellij reviewer 123456\n");
        assert_eq!(commands, vec![AcpSpecialCommand::EnsureRoom]);
    }

    #[test]
    fn detects_bare_acp_commands_for_zellij_prefixing() {
        let mut telemetry = RoomTelemetry::default();
        let commands = telemetry.ingest_input_bytes(PaneId::Terminal(1), b"review request 123\n");
        assert_eq!(commands, vec![AcpSpecialCommand::PrefixWithZellij]);
    }

    #[test]
    fn detects_provider_launches_across_segmented_input_and_enter() {
        let mut telemetry = RoomTelemetry::default();
        let first = telemetry.ingest_input_bytes(PaneId::Terminal(1), b"cla");
        let second = telemetry.ingest_input_bytes(PaneId::Terminal(1), b"ude");
        let third = telemetry.ingest_input_bytes(PaneId::Terminal(1), b"\r");

        assert!(first.is_empty());
        assert!(second.is_empty());
        assert_eq!(
            third,
            vec![AcpSpecialCommand::LaunchProvider {
                provider: "claude".to_owned()
            }]
        );
    }
}
