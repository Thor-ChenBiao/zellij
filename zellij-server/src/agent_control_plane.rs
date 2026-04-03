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

#[derive(Default)]
pub(crate) struct RoomTelemetry {
    pane_viewport_hashes: HashMap<PaneId, u64>,
    pty_line_buffers: HashMap<u32, String>,
}

impl RoomTelemetry {
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
                if let Some(file_hint) = extract_file_hint(trimmed) {
                    emissions.push(StreamEmission {
                        stream: ViewStreamKind::Code,
                        kind: "file_hint".to_owned(),
                        message: format!("Detected file hint {}", file_hint),
                        payload: json!({
                            "terminal_id": terminal_id,
                            "file_path": file_hint,
                            "source_line": trimmed,
                        }),
                    });
                }
            } else if let Some(file_hint) = extract_file_hint(trimmed) {
                emissions.push(StreamEmission {
                    stream: ViewStreamKind::Code,
                    kind: "file_hint".to_owned(),
                    message: format!("Detected file hint {}", file_hint),
                    payload: json!({
                        "terminal_id": terminal_id,
                        "file_path": file_hint,
                        "source_line": trimmed,
                    }),
                });
            }
        }
        if buffer.len() > 4096 {
            let keep_from = suffix_start_for_max_bytes(buffer, 1024);
            buffer.drain(..keep_from);
        }
        emissions
    }

    pub fn pane_render_emissions(&mut self, report: &PaneRenderReport) -> Vec<StreamEmission> {
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
            emissions.push(StreamEmission {
                stream: ViewStreamKind::Code,
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
}

fn pane_contents_payload(pane_id: PaneId, pane_contents: &PaneContents) -> serde_json::Value {
    let preview: Vec<String> = pane_contents.viewport.iter().take(40).cloned().collect();
    json!({
        "pane_id": pane_id.to_string(),
        "viewport": preview,
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
    fn extracts_tool_calls_and_file_hints_from_terminal_lines() {
        let mut telemetry = RoomTelemetry::default();
        let emissions = telemetry.ingest_pty_bytes(
            7,
            b"\xe2\x8f\xba Write(src/main.rs)\n\xe2\x8f\xba Bash(cargo test -p zellij-server)\n",
        );

        assert_eq!(emissions.len(), 3);
        assert_eq!(emissions[0].kind, "tool_call");
        assert_eq!(emissions[1].kind, "file_hint");
        assert_eq!(emissions[2].kind, "tool_call");
        assert_eq!(emissions[2].stream, ViewStreamKind::Events);
        assert!(emissions[2].payload.get("file_path").is_none());
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

        assert_eq!(first.len(), 1);
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
}
