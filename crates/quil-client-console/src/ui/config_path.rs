use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{app::App, i18n::t};

use super::{chrome, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.config_path_in_flight {
        t(locale, UiText::ValidatingCtrlCExit)
    } else if app.config_path_required {
        t(locale, UiText::EnterValidateEscStayCtrlCExit)
    } else {
        t(locale, UiText::EnterValidateEscBackCtrlCExit)
    };
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::ConfigurationDirectoryTitle),
        app,
        footer,
    ) else {
        return;
    };
    let inner = screen.body.inner(ratatui::layout::Margin::new(1, 1));
    frame.render_widget(
        chrome::panel(t(locale, UiText::ConfigurationDirectory)),
        screen.body,
    );
    let sections = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(2),
    ])
    .split(inner);
    let input_state = if app.config_path_in_flight {
        Line::from(vec![
            Span::styled(t(locale, UiText::ValidatingInputLocked), theme::attention()),
            Span::styled(
                t(locale, UiText::RequiresConfigYmlAndKeysYml),
                theme::muted(),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(t(locale, UiText::Editing), theme::focus()),
            Span::styled(
                t(locale, UiText::RequiresConfigYmlAndKeysYml),
                theme::muted(),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(input_state), sections[0]);

    let path_width = sections[1].width.saturating_sub(2) as usize;
    let path_height = sections[1].height.saturating_sub(2) as usize;
    let visible_path = visible_path_lines(&app.config_path_input, path_width, path_height);
    frame.render_widget(Paragraph::new(visible_path), sections[1]);

    let status = if app.config_path_in_flight {
        t(locale, UiText::ProcessingConfigurationDirectory)
    } else {
        app.config_path_error
            .as_deref()
            .unwrap_or(t(locale, UiText::EnterAndValidateAConfigurationDirectory))
    };
    let status_style = if !app.config_path_in_flight && app.config_path_error.is_some() {
        theme::failure()
    } else {
        theme::muted()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(status, status_style))),
        sections[2],
    );
}

fn visible_path_lines(path: &str, max_width: usize, max_height: usize) -> Vec<Line<'static>> {
    if max_width == 0 || max_height == 0 {
        return Vec::new();
    }

    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for grapheme in path.graphemes(true) {
        if grapheme == "\n" {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
            continue;
        }
        let width = grapheme.width();
        if line_width + width > max_width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        line.push_str(grapheme);
        line_width += width;
    }
    lines.push(line);

    let truncated = lines.len() > max_height;
    let first_visible = lines.len().saturating_sub(max_height);
    let mut visible = lines.split_off(first_visible);
    if truncated {
        let first = &mut visible[0];
        while UnicodeWidthStr::width(first.as_str()) + 1 > max_width {
            let next_boundary = first
                .grapheme_indices(true)
                .nth(1)
                .map_or(first.len(), |v| v.0);
            first.drain(..next_boundary);
        }
        first.insert(0, '…');
    }

    visible.into_iter().map(Line::from).collect()
}
