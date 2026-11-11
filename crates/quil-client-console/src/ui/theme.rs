use ratatui::style::{Color, Modifier, Style};

pub const ACCENT: Color = Color::Cyan;
pub const MUTED: Color = Color::DarkGray;
pub const FOCUS_MARKER: &str = "› ";

pub fn focus() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn healthy() -> Style {
    Style::default().fg(Color::Green)
}

pub fn attention() -> Style {
    Style::default().fg(Color::Yellow)
}

pub fn failure() -> Style {
    Style::default().fg(Color::Red)
}

pub fn label() -> Style {
    Style::default().fg(Color::White)
}

pub fn title() -> Style {
    focus()
}

pub fn selected() -> Style {
    focus()
}

pub fn muted() -> Style {
    Style::default().fg(MUTED)
}

pub fn normal() -> Style {
    Style::default()
}

pub fn success() -> Style {
    healthy()
}
