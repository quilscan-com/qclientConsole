use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{App, ConfirmChoice, Page, CAPABILITY_GROUPS},
    capability::{CapabilityFieldKind, CapabilityGroup, CapabilityTaskState},
    i18n::{t, UiText},
};

use super::{
    chrome::{self, Breakpoint},
    text::{truncate_cells, wrap_cells},
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    match app.page {
        Page::Capabilities => draw_center(frame, app),
        Page::CapabilityForm => draw_form(frame, app),
        Page::CapabilityConfirm => draw_confirm(frame, app),
        Page::CapabilityTask => draw_task(frame, app),
        _ => {}
    }
}

fn group_label(app: &App, group: CapabilityGroup) -> &'static str {
    match group {
        CapabilityGroup::Node => t(app.locale(), UiText::NodeManagement),
        CapabilityGroup::Prover => t(app.locale(), UiText::ProverShards),
        CapabilityGroup::Wallet => t(app.locale(), UiText::WalletToken),
        CapabilityGroup::Identity => t(app.locale(), UiText::KeysAddresses),
        CapabilityGroup::Data => t(app.locale(), UiText::DataApplications),
        CapabilityGroup::Diagnostics => t(app.locale(), UiText::DiagnosticsLogs),
        CapabilityGroup::System => t(app.locale(), UiText::Other),
    }
}

fn draw_center(frame: &mut Frame<'_>, app: &App) {
    let Some(screen) = chrome::screen(
        frame,
        t(app.locale(), UiText::QclientCommands),
        app,
        t(app.locale(), UiText::CapabilityListFooter),
    ) else {
        return;
    };
    let [groups, body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(screen.body);
    draw_groups(frame, app, groups);
    match screen.breakpoint {
        Breakpoint::Wide | Breakpoint::Standard => {
            let [list, detail] =
                Layout::horizontal([Constraint::Percentage(38), Constraint::Min(0)]).areas(body);
            draw_capability_list(frame, app, list);
            draw_capability_detail(frame, app, detail);
        }
        Breakpoint::Narrow => {
            let detail_height = body.height.saturating_sub(3).min(8);
            let [list, detail] =
                Layout::vertical([Constraint::Min(3), Constraint::Length(detail_height)])
                    .areas(body);
            draw_capability_list(frame, app, list);
            draw_compact_detail(frame, app, detail);
        }
    }
}

fn capability_impact(app: &App, read_only: bool) -> &str {
    if read_only {
        t(app.locale(), UiText::ReadOnly)
    } else {
        t(app.locale(), UiText::ReadWriteNodeOperation)
    }
}

fn field_kind(kind: CapabilityFieldKind) -> &'static str {
    match kind {
        CapabilityFieldKind::Text => "text",
        CapabilityFieldKind::Path => "path",
        CapabilityFieldKind::Integer => "integer",
        CapabilityFieldKind::Boolean => "boolean",
    }
}

fn field_summary(app: &App, field: &crate::capability::CapabilityField) -> String {
    let mut attributes = vec![field_kind(field.kind)];
    if field.required {
        attributes.push(t(app.locale(), UiText::Required));
    }
    if field.repeated {
        attributes.push(t(app.locale(), UiText::Repeated));
    }
    format!("{} · {}", field.label, attributes.join(" · "))
}

fn draw_compact_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(capability) = app.capabilities.selected() else {
        return;
    };
    let width = usize::from(area.width).max(1);
    let mut lines = vec![
        Line::raw(truncate_cells(&capability.description, width)),
        Line::from(vec![
            Span::styled(
                format!("{}: ", t(app.locale(), UiText::Impact)),
                theme::muted(),
            ),
            Span::raw(capability_impact(app, capability.read_only)),
        ]),
        Line::styled(
            format!(
                "{}: {}",
                t(app.locale(), UiText::Fields),
                capability.fields.len()
            ),
            theme::muted(),
        ),
    ];
    if capability.fields.is_empty() {
        lines.push(Line::styled(
            t(app.locale(), UiText::NoData),
            theme::muted(),
        ));
    } else {
        lines.extend(capability.fields.iter().map(|field| {
            Line::styled(
                truncate_cells(
                    &format!("{} — {}", field_summary(app, field), field.help),
                    width,
                ),
                theme::muted(),
            )
        }));
    }
    let scroll = app
        .capabilities
        .detail_scroll
        .min(lines.len().saturating_sub(usize::from(area.height)))
        .min(u16::MAX as usize) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn draw_groups(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut spans = Vec::new();
    for (index, group) in CAPABILITY_GROUPS.iter().copied().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", theme::muted()));
        }
        spans.push(Span::styled(
            group_label(app, group),
            if index == app.capabilities.group_index {
                theme::focus()
            } else {
                theme::muted()
            },
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_capability_list(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let items = app.capabilities.group_items();
    let rows = items.iter().map(|capability| {
        ListItem::new(truncate_cells(
            &format!("{}  {}", capability.title, capability.id),
            usize::from(area.width.saturating_sub(4)),
        ))
    });
    let selected = (!items.is_empty()).then_some(
        app.capabilities
            .item_index
            .min(items.len().saturating_sub(1)),
    );
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(
        List::new(rows)
            .block(chrome::focus_panel(
                group_label(app, app.capabilities.group()),
                true,
            ))
            .highlight_style(theme::focus())
            .highlight_symbol(theme::FOCUS_MARKER),
        area,
        &mut state,
    );
}

fn draw_capability_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(capability) = app.capabilities.selected() else {
        frame.render_widget(
            Paragraph::new(t(app.locale(), UiText::NoData))
                .block(chrome::panel(t(app.locale(), UiText::FeatureDetails))),
            area,
        );
        return;
    };
    let mode = capability_impact(app, capability.read_only);
    let inner_width = usize::from(area.width.saturating_sub(2)).max(1);
    let mut lines = vec![
        Line::styled(
            truncate_cells(&capability.title, inner_width),
            theme::label(),
        ),
        Line::styled(truncate_cells(&capability.id, inner_width), theme::muted()),
        Line::raw(""),
    ];
    lines.extend(
        wrap_cells(&capability.description, inner_width)
            .into_iter()
            .map(Line::raw),
    );
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        truncate_cells(
            &format!("{}: {mode}", t(app.locale(), UiText::Impact)),
            inner_width,
        ),
        theme::muted(),
    ));
    lines.push(Line::styled(
        truncate_cells(
            &format!(
                "{}: {}",
                t(app.locale(), UiText::Fields),
                capability.fields.len()
            ),
            inner_width,
        ),
        theme::muted(),
    ));
    for field in &capability.fields {
        lines.push(Line::styled(
            truncate_cells(
                &format!("{} — {}", field_summary(app, field), field.help),
                inner_width,
            ),
            theme::muted(),
        ));
    }
    if let Some(error) = &app.capabilities.error {
        lines.push(Line::raw(""));
        lines.extend(
            wrap_cells(error, inner_width)
                .into_iter()
                .map(|line| Line::styled(line, theme::failure())),
        );
    }
    let scroll = app
        .capabilities
        .detail_scroll
        .min(
            lines
                .len()
                .saturating_sub(usize::from(area.height.saturating_sub(2))),
        )
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome::panel(t(app.locale(), UiText::FeatureDetails)))
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_form(frame: &mut Frame<'_>, app: &App) {
    let Some(screen) = chrome::screen(
        frame,
        t(app.locale(), UiText::QclientCommands),
        app,
        t(app.locale(), UiText::CapabilityFormFooter),
    ) else {
        return;
    };
    let Some(capability) = app.capabilities.current() else {
        frame.render_widget(Paragraph::new(t(app.locale(), UiText::NoData)), screen.body);
        return;
    };
    let [fields_area, help_area] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(4)]).areas(screen.body);
    let mut rows = capability
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let mut meta = Vec::new();
            if field.required {
                meta.push(t(app.locale(), UiText::Required));
            }
            if field.repeated {
                meta.push(t(app.locale(), UiText::Repeated));
            }
            let value = field_display_value(app, field);
            let edit = if app.capabilities.editing && app.capabilities.field_index == index {
                "✎ "
            } else {
                "  "
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!("{edit}{}", field.label), theme::label()),
                    Span::styled(
                        if meta.is_empty() {
                            String::new()
                        } else {
                            format!("  [{}]", meta.join(", "))
                        },
                        theme::muted(),
                    ),
                ]),
                Line::styled(format!("    {value}"), theme::normal()),
            ])
        })
        .collect::<Vec<_>>();
    rows.push(ListItem::new(Line::styled(
        t(app.locale(), UiText::Submit),
        theme::label(),
    )));
    let mut state = ListState::default().with_selected(Some(
        app.capabilities.field_index.min(capability.fields.len()),
    ));
    frame.render_stateful_widget(
        List::new(rows)
            .block(chrome::focus_panel(capability.title.clone(), true))
            .highlight_style(theme::focus())
            .highlight_symbol(theme::FOCUS_MARKER),
        fields_area,
        &mut state,
    );
    draw_form_help(frame, app, help_area);
    draw_error(frame, app, fields_area);
}

fn draw_form_help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(capability) = app.capabilities.current() else {
        return;
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{}: ", t(app.locale(), UiText::Impact)),
            theme::muted(),
        ),
        Span::raw(capability_impact(app, capability.read_only)),
    ])];
    if let Some(field) = capability.fields.get(app.capabilities.field_index) {
        lines.push(Line::styled(field_summary(app, field), theme::label()));
        lines.extend(
            wrap_cells(&field.help, usize::from(area.width).max(1))
                .into_iter()
                .take(2)
                .map(|line| Line::styled(line, theme::muted())),
        );
    } else {
        lines.push(Line::styled(
            t(app.locale(), UiText::Submit),
            theme::label(),
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn field_display_value(app: &App, field: &crate::capability::CapabilityField) -> String {
    let values = app.capabilities.values.get(&field.id).unwrap_or_default();
    if field.sensitive && values.iter().any(|value| !value.is_empty()) {
        return t(app.locale(), UiText::SensitiveValueHidden).into();
    }
    let value = values.join("  |  ");
    if field.kind == CapabilityFieldKind::Boolean && value.is_empty() {
        "false".into()
    } else if value.is_empty() {
        "—".into()
    } else {
        value
    }
}

fn draw_error(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some(error) = &app.capabilities.error else {
        return;
    };
    let inner = area.inner(Margin::new(2, 1));
    let height = 2.min(inner.height);
    let row = Rect::new(
        inner.x,
        inner.bottom().saturating_sub(height),
        inner.width,
        height,
    );
    frame.render_widget(
        Paragraph::new(error.as_str())
            .style(theme::failure())
            .wrap(Wrap { trim: true }),
        row,
    );
}

fn draw_confirm(frame: &mut Frame<'_>, app: &App) {
    let Some(screen) = chrome::screen(
        frame,
        t(app.locale(), UiText::Confirm),
        app,
        t(app.locale(), UiText::CapabilityConfirmFooter),
    ) else {
        return;
    };
    let Some(capability) = app.capabilities.current() else {
        frame.render_widget(Paragraph::new(t(app.locale(), UiText::NoData)), screen.body);
        return;
    };
    let [details, choices] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).areas(screen.body);
    let mut lines = vec![
        Line::styled(capability.title.clone(), theme::label()),
        Line::styled(capability.id.clone(), theme::muted()),
        Line::from(vec![
            Span::styled(
                format!("{}: ", t(app.locale(), UiText::Impact)),
                theme::muted(),
            ),
            Span::raw(capability_impact(app, capability.read_only)),
        ]),
    ];
    for field in &capability.fields {
        lines.push(Line::from(vec![
            Span::styled(format!("{}: ", field.label), theme::muted()),
            Span::raw(field_display_value(app, field)),
        ]));
    }
    if let Some(message) = &capability.confirmation {
        lines.push(Line::raw(""));
        lines.extend(
            wrap_cells(message, usize::from(details.width).max(1))
                .into_iter()
                .map(Line::raw),
        );
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), details);
    let cancel = t(app.locale(), UiText::Cancel);
    let confirm = t(app.locale(), UiText::Confirm);
    let choices_line = match app.capabilities.confirmation {
        ConfirmChoice::Cancel => Line::from(vec![
            Span::styled(format!("[ {cancel} ]"), theme::focus()),
            Span::raw(format!("   {confirm}")),
        ]),
        ConfirmChoice::Confirm => Line::from(vec![
            Span::raw(format!("  {cancel}   ")),
            Span::styled(format!("[ {confirm} ]"), theme::focus()),
        ]),
    };
    frame.render_widget(
        Paragraph::new(choices_line).alignment(Alignment::Center),
        choices,
    );
}

fn draw_task(frame: &mut Frame<'_>, app: &App) {
    let Some(screen) = chrome::screen(
        frame,
        t(app.locale(), UiText::QclientCommands),
        app,
        t(app.locale(), UiText::CapabilityTaskFooter),
    ) else {
        return;
    };
    let mut lines = Vec::new();
    if let Some(task) = &app.capabilities.current_task {
        let (status, style) = match task.state {
            CapabilityTaskState::Running => (t(app.locale(), UiText::Running), theme::attention()),
            CapabilityTaskState::Succeeded => (t(app.locale(), UiText::Success), theme::success()),
            CapabilityTaskState::Failed => (t(app.locale(), UiText::Failed), theme::failure()),
        };
        lines.push(Line::styled(task.capability_id.clone(), theme::label()));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}: ", t(app.locale(), UiText::Status)),
                theme::muted(),
            ),
            Span::styled(status, style),
        ]));
        lines.push(Line::from(format!(
            "{}: {}",
            t(app.locale(), UiText::ExitCode),
            task.exit_code
                .map_or_else(|| "—".into(), |value| value.to_string())
        )));
        append_output(
            &mut lines,
            t(app.locale(), UiText::StandardOutput),
            &task.stdout,
            usize::from(screen.body.width.saturating_sub(2)).max(1),
        );
        append_output(
            &mut lines,
            t(app.locale(), UiText::StandardError),
            &task.stderr,
            usize::from(screen.body.width.saturating_sub(2)).max(1),
        );
    } else if let Some(error) = &app.capabilities.error {
        lines.push(Line::styled(error.clone(), theme::failure()));
    } else {
        lines.push(Line::styled(
            t(app.locale(), UiText::Queued),
            theme::attention(),
        ));
    }
    let max_scroll = lines
        .len()
        .saturating_sub(usize::from(screen.body.height.saturating_sub(2)));
    let scroll = app
        .capabilities
        .task_scroll
        .min(max_scroll)
        .min(u16::MAX as usize) as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .block(chrome::panel(t(app.locale(), UiText::Output)))
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false }),
        screen.body,
    );
}

fn append_output(lines: &mut Vec<Line<'static>>, title: &str, output: &str, width: usize) {
    lines.push(Line::raw(""));
    lines.push(Line::styled(title.to_owned(), theme::label()));
    if output.is_empty() {
        lines.push(Line::styled("—", theme::muted()));
    } else {
        lines.extend(wrap_cells(output, width).into_iter().map(Line::raw));
    }
}
