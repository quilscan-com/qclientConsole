use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{app::App, i18n::t};

use super::{
    text::{compact_middle, truncate_cells},
    theme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    Narrow,
    Standard,
    Wide,
}

impl Breakpoint {
    pub fn from_width(width: u16) -> Self {
        match width {
            0..=69 => Self::Narrow,
            70..=99 => Self::Standard,
            _ => Self::Wide,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenAreas {
    pub header: Rect,
    pub body: Rect,
    pub footer: Rect,
    pub breakpoint: Breakpoint,
}

/// Renders shared chrome; pages own the returned body without an outer border.
pub fn screen(frame: &mut Frame<'_>, title: &str, app: &App, footer: &str) -> Option<ScreenAreas> {
    let area = frame.area();
    if area.width < 42 || area.height < 12 {
        let message = t(
            app.locale(),
            UiText::MinimumSize4212WindowTooSmallResizeTheTerminalCurrent,
        );
        frame.render_widget(
            Paragraph::new(format!("{message} {}×{}", area.width, area.height))
                .style(theme::attention())
                .wrap(Wrap { trim: true }),
            area,
        );
        return None;
    }

    let [header, body, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    let locale = app.locale();
    let (status, status_style) = match (&app.snapshot, &app.error, app.loading) {
        (Some(_), _, true) => (t(locale, UiText::RefreshingStale), theme::attention()),
        (None, _, true) => (t(locale, UiText::Loading), theme::attention()),
        (Some(_), Some(_), false) => (t(locale, UiText::Stale), theme::attention()),
        (None, Some(_), false) => (t(locale, UiText::LoadFailedSentenceCase), theme::failure()),
        (Some(node), None, false) if node.connected => {
            (t(locale, UiText::Connected), theme::healthy())
        }
        (Some(_), None, false) => (t(locale, UiText::Disconnected), theme::failure()),
        (None, None, false) => (t(locale, UiText::NotLoaded), theme::muted()),
    };
    let width = usize::from(area.width);
    let content_width = width.saturating_sub(status.width() + 6);
    let title = truncate_cells(title, content_width.min(width / 3));
    let config = app
        .active_config()
        .map_or(t(locale, UiText::NoConfigurationSelected), |entry| {
            entry.name.as_str()
        });
    let config = truncate_cells(config, content_width.saturating_sub(title.width()));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(title, theme::title()),
            Span::styled(" · ", theme::muted()),
            Span::styled(config, theme::label()),
            Span::styled(" · ", theme::muted()),
            Span::styled(status, status_style),
        ])),
        header,
    );
    frame.render_widget(
        Paragraph::new(compact_middle(footer, width)).style(theme::muted()),
        footer_area,
    );

    Some(ScreenAreas {
        header,
        body,
        footer: footer_area,
        breakpoint: Breakpoint::from_width(area.width),
    })
}

pub fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme::muted())
        .title(title.into().style(theme::label()))
}

pub fn focus_panel(title: impl Into<Line<'static>>, focused: bool) -> Block<'static> {
    let style = if focused {
        theme::focus()
    } else {
        theme::muted()
    };
    let title_style = if focused {
        theme::focus()
    } else {
        theme::label()
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(title.into().style(title_style))
}
