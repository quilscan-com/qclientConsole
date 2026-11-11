use crate::i18n::UiText;
use ratatui::{
    layout::Margin,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{app::App, i18n::t};
use unicode_width::UnicodeWidthStr;

use super::{chrome, text, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let state = &app.other;
    let locale = app.locale();
    let footer = if state.in_flight || state.read_rpc_in_flight {
        t(locale, UiText::SavingEscQBackCtrlCExit)
    } else if state.editing_port || state.editing_read_rpc {
        t(locale, UiText::EnterSaveEscCancelCtrlCExit)
    } else if frame.area().width < 50 {
        t(locale, UiText::EnterEscQBackHelpCtrlCExit)
    } else {
        t(locale, UiText::SelectEnterEditEscQBackHelpCtrlCExit)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::Other), app, footer) else {
        return;
    };
    frame.render_widget(
        chrome::panel(t(locale, UiText::ConsoleSettings)),
        screen.body,
    );
    let inner = screen.body.inner(Margin::new(1, 1));
    let width = usize::from(inner.width);
    let port_label = t(locale, UiText::WebPort);
    let port_width = width.saturating_sub(port_label.width() + 4);
    let port = if state.editing_port {
        text::tail_cells(&state.port_input, port_width)
    } else {
        state.settings.port.to_string()
    };
    let read_rpc_label = t(locale, UiText::ReadRpc);
    let read_rpc_width = width.saturating_sub(read_rpc_label.width() + 4);
    let read_rpc = if state.editing_read_rpc {
        text::tail_cells(&state.read_rpc_input, read_rpc_width)
    } else if let Some(value) = &state.read_rpc_override {
        value.clone()
    } else if state.effective_read_rpc.is_empty() {
        "—".into()
    } else {
        format!(
            "{} ({})",
            state.effective_read_rpc,
            t(locale, UiText::ReadRpcDefault)
        )
    };
    let rows = [
        format!(
            "{}  {}",
            t(locale, UiText::WebManagement),
            t(
                locale,
                if state.settings.enabled {
                    UiText::Enabled
                } else {
                    UiText::Disabled
                }
            )
        ),
        format!("{port_label}  {port}"),
        format!("{read_rpc_label}  {read_rpc}"),
        format!(
            "{}  {}  ←/→",
            t(locale, UiText::Language),
            locale.display_name(),
        ),
    ];
    let mut lines = rows
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let selected = state.selected == index;
            Line::from(Span::styled(
                text::truncate_cells(
                    &format!(
                        "{}{}",
                        if selected { theme::FOCUS_MARKER } else { "  " },
                        row
                    ),
                    width,
                ),
                if selected {
                    theme::focus()
                } else {
                    theme::normal()
                },
            ))
        })
        .collect::<Vec<_>>();
    lines.push(Line::default());
    if state.in_flight || state.read_rpc_in_flight {
        lines.push(Line::styled(t(locale, UiText::Saving), theme::attention()));
    } else if let Some(error) = &state.error {
        lines.extend(
            text::wrap_cells(&format!("{}: {error}", t(locale, UiText::Error)), width)
                .into_iter()
                .map(|line| Line::styled(line, theme::failure())),
        );
    } else if state.editing_port {
        lines.extend(
            text::wrap_cells(t(locale, UiText::PortInputHint), width)
                .into_iter()
                .map(|line| Line::styled(line, theme::muted())),
        );
    } else if state.editing_read_rpc {
        lines.extend(
            text::wrap_cells(t(locale, UiText::ReadRpcInputHint), width)
                .into_iter()
                .map(|line| Line::styled(line, theme::muted())),
        );
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
