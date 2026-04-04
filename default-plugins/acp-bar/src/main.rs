use ansi_term::{
    Color::{Fixed, RGB},
    Style,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use unicode_width::UnicodeWidthStr;
use zellij_tile::prelude::*;

const ACP_ROOM_STATE_PIPE: &str = "acp-room-state";

#[derive(Default)]
struct State {
    room_id: String,
    self_provider: String,
    self_role: String,
    driver_count: usize,
    reviewer_count: usize,
    human_count: usize,
    auto_review_enabled: bool,
    auto_send_enabled: bool,
    mode_info: ModeInfo,
}

#[derive(Default, Deserialize)]
struct RoomStatePayload {
    room_id: Option<String>,
    self_provider: Option<String>,
    self_role: Option<String>,
    driver_count: Option<usize>,
    reviewer_count: Option<usize>,
    human_count: Option<usize>,
    auto_review_enabled: Option<bool>,
    auto_send_enabled: Option<bool>,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        self.room_id = configuration.get("room_id").cloned().unwrap_or_default();
        self.self_provider = configuration
            .get("self_provider")
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned());
        self.self_role = configuration
            .get("self_role")
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned());
        self.driver_count = parse_count(&configuration, "driver_count");
        self.reviewer_count = parse_count(&configuration, "reviewer_count");
        self.human_count = parse_count(&configuration, "human_count");
        self.auto_review_enabled = parse_bool(&configuration, "auto_review_enabled");
        self.auto_send_enabled = parse_bool(&configuration, "auto_send_enabled");
        set_selectable(true);
        subscribe(&[EventType::ModeUpdate]);
        rename_plugin_pane(get_plugin_ids().plugin_id, "ACP");
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::ModeUpdate(mode_info) => {
                if self.mode_info != mode_info {
                    self.mode_info = mode_info;
                    true
                } else {
                    false
                }
            },
            _ => false,
        }
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        if pipe_message.name != ACP_ROOM_STATE_PIPE {
            return false;
        }
        let Some(payload) = pipe_message.payload else {
            return false;
        };
        let Ok(payload) = serde_json::from_str::<RoomStatePayload>(&payload) else {
            return false;
        };
        if let Some(room_id) = payload.room_id {
            self.room_id = room_id;
        }
        if let Some(self_provider) = payload.self_provider {
            self.self_provider = self_provider;
        }
        if let Some(self_role) = payload.self_role {
            self.self_role = self_role;
        }
        if let Some(driver_count) = payload.driver_count {
            self.driver_count = driver_count;
        }
        if let Some(reviewer_count) = payload.reviewer_count {
            self.reviewer_count = reviewer_count;
        }
        if let Some(human_count) = payload.human_count {
            self.human_count = human_count;
        }
        if let Some(auto_review_enabled) = payload.auto_review_enabled {
            self.auto_review_enabled = auto_review_enabled;
        }
        if let Some(auto_send_enabled) = payload.auto_send_enabled {
            self.auto_send_enabled = auto_send_enabled;
        }
        true
    }

    fn render(&mut self, _rows: usize, cols: usize) {
        let background = self.mode_info.style.colors.text_unselected.background;
        let text = self.mode_info.style.colors.text_unselected.base;
        let accent_background = self.mode_info.style.colors.ribbon_selected.background;
        let accent_text = self.mode_info.style.colors.ribbon_selected.base;

        let background = to_ansi_color(background);
        let text = to_ansi_color(text);
        let accent_background = to_ansi_color(accent_background);
        let accent_text = to_ansi_color(accent_text);

        let has_room = !self.room_id.trim().is_empty();
        let prefix = has_room.then(|| {
            Style::new()
                .fg(accent_text)
                .on(accent_background)
                .bold()
                .paint(" ACP ")
        });
        let prefix_width = if has_room { 5 } else { 0 };
        let body = self.render_body(cols.saturating_sub(prefix_width));
        let body = Style::new().fg(text).on(background).paint(if has_room {
            format!(" {}", body)
        } else {
            body
        });

        match background {
            RGB(r, g, b) => {
                if let Some(prefix) = prefix {
                    print!("{}{}", prefix, body);
                } else {
                    print!("{}", body);
                }
                print!("\u{1b}[48;2;{};{};{}m\u{1b}[0K", r, g, b);
            },
            Fixed(color) => {
                if let Some(prefix) = prefix {
                    print!("{}{}", prefix, body);
                } else {
                    print!("{}", body);
                }
                print!("\u{1b}[48;5;{}m\u{1b}[0K", color);
            },
            _ => {
                if let Some(prefix) = prefix {
                    print!("{}{}", prefix, body);
                } else {
                    print!("{}", body);
                }
                print!("\u{1b}[0K");
            },
        }
    }
}

impl State {
    fn render_body(&self, available_cols: usize) -> String {
        if self.room_id.trim().is_empty() {
            return String::new();
        }
        let full = format!(
            "room: {}{} | roles: {} | auto: {}",
            self.room_id,
            self.render_self_section(false),
            self.role_summary(false),
            self.automation_summary(false)
        );
        if display_width(&full) <= available_cols {
            return full;
        }

        let compact = format!(
            "{}{} | {} | {}",
            self.room_id,
            self.render_self_section(true),
            self.role_summary(true),
            self.automation_summary(true)
        );
        if display_width(&compact) <= available_cols {
            return compact;
        }

        truncate_display(&format!("room: {}", self.room_id), available_cols)
    }

    fn render_self_section(&self, compact: bool) -> String {
        if self.self_provider.trim().is_empty() || self.self_role.trim().is_empty() {
            return String::new();
        }
        if compact {
            format!(" | {}/{}", self.self_provider, self.self_role)
        } else {
            format!(" | self: {}/{}", self.self_provider, self.self_role)
        }
    }

    fn role_summary(&self, compact: bool) -> String {
        let mut roles = vec![if compact {
            format!("D{}", self.driver_count)
        } else {
            format!("driver({})", self.driver_count)
        }];
        roles.push(if compact {
            format!("R{}", self.reviewer_count)
        } else {
            format!("reviewer({})", self.reviewer_count)
        });
        if self.human_count > 0 {
            roles.push(if compact {
                format!("H{}", self.human_count)
            } else {
                format!("human({})", self.human_count)
            });
        }
        roles.join(" ")
    }

    fn automation_summary(&self, compact: bool) -> String {
        if compact {
            format!(
                "AR:{} AS:{}",
                if self.auto_review_enabled {
                    "on"
                } else {
                    "off"
                },
                if self.auto_send_enabled { "on" } else { "off" }
            )
        } else {
            format!(
                "review:{} send:{}",
                if self.auto_review_enabled {
                    "on"
                } else {
                    "off"
                },
                if self.auto_send_enabled { "on" } else { "off" }
            )
        }
    }
}

fn parse_count(configuration: &BTreeMap<String, String>, key: &str) -> usize {
    configuration
        .get(key)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
}

fn parse_bool(configuration: &BTreeMap<String, String>, key: &str) -> bool {
    configuration
        .get(key)
        .map(|value| value == "true" || value == "1")
        .unwrap_or(false)
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn truncate_display(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_owned();
    }
    if max_width <= 3 {
        return text.chars().take(max_width).collect();
    }
    let mut current = String::new();
    let target = max_width.saturating_sub(3);
    for ch in text.chars() {
        let candidate = format!("{}{}", current, ch);
        if display_width(&candidate) > target {
            break;
        }
        current.push(ch);
    }
    current.push_str("...");
    current
}

fn to_ansi_color(color: PaletteColor) -> ansi_term::Color {
    match color {
        PaletteColor::Rgb((r, g, b)) => RGB(r, g, b),
        PaletteColor::EightBit(color) => Fixed(color),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_body_is_blank_without_room() {
        let state = State::default();

        assert_eq!(state.render_body(80), "");
    }

    #[test]
    fn render_body_falls_back_to_compact_format_when_width_is_tight() {
        let state = State {
            room_id: "123456".to_owned(),
            self_provider: "claude".to_owned(),
            self_role: "driver".to_owned(),
            driver_count: 1,
            reviewer_count: 2,
            human_count: 0,
            auto_review_enabled: true,
            auto_send_enabled: false,
            mode_info: ModeInfo::default(),
        };

        let rendered = state.render_body(48);

        assert!(rendered.contains("123456"));
        assert!(rendered.contains("claude/driver"));
        assert!(rendered.contains("D1 R2"));
        assert!(rendered.contains("AR:on AS:off"));
    }
}
