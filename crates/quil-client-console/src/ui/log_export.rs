use crate::i18n::UiText;
use ratatui::{
    layout::Margin,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{
    app::{App, ConfirmChoice, LOG_EXPORT_WINDOWS},
    domain::LogExportLevel,
    i18n::{t, Locale},
};

use super::{chrome, text::truncate_cells, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.log_export.in_flight {
        t(locale, UiText::WorkingCtrlCQuit)
    } else if app.log_export.preview.is_some() {
        t(locale, UiText::SelectEnterConfirmEscQCancelCtrlCQuit)
    } else {
        t(locale, UiText::FieldAdjustEnterPreviewEscQBackCtrlCQuit)
    };
    let Some(screen) = chrome::screen(
        frame,
        t(locale, UiText::RedactedLogExportTitle),
        app,
        footer,
    ) else {
        return;
    };
    frame.render_widget(
        chrome::panel(t(locale, UiText::RedactedLogExport)),
        screen.body,
    );
    let inner = screen.body.inner(Margin::new(1, 1));

    if app.log_export.in_flight {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::ScanningRedactingAndPreparingAPreview))
                .style(theme::attention()),
            inner,
        );
    } else if let Some(preview) = &app.log_export.preview {
        draw_preview(frame, app, preview, inner);
    } else {
        draw_settings(frame, app, inner);
    }
}

fn draw_settings(frame: &mut Frame<'_>, app: &App, area: ratatui::layout::Rect) {
    let locale = app.locale();
    let window = window_label(LOG_EXPORT_WINDOWS[app.log_export.window_index], locale);
    let level = LogExportLevel::ALL[app.log_export.level_index].label();
    let mut lines = Vec::new();
    if let Some(error) = &app.log_export.error {
        lines.push(Line::from(Span::styled(
            truncate_cells(
                &crate::i18n::localize_service_error(locale, error),
                usize::from(area.width),
            ),
            theme::failure(),
        )));
    } else {
        lines.push(Line::from(t(
            locale,
            UiText::APreviewIsGeneratedBeforeAnyFileIsWritten,
        )));
    }
    lines.extend([
        Line::from(""),
        option_line(
            t(locale, UiText::TimeRange),
            window,
            app.log_export.field_index == 0,
        ),
        option_line(
            t(locale, UiText::MinimumLevel),
            level,
            app.log_export.field_index == 1,
        ),
        Line::from(""),
        Line::from(t(
            locale,
            UiText::UpTo24HoursWarnErrorLinesAreRetainedForContext,
        )),
        Line::from(Span::styled(
            t(
                locale,
                UiText::TheExportArchiveMayBeSharedMapTxtContainsOriginalValuesAndMustNeverBeShared,
            ),
            theme::attention(),
        )),
    ]);
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_preview(
    frame: &mut Frame<'_>,
    app: &App,
    preview: &crate::domain::LogExportPreview,
    area: ratatui::layout::Rect,
) {
    let locale = app.locale();
    let width = usize::from(area.width);
    let redactions = if preview.redactions.is_empty() {
        t(locale, UiText::None).into()
    } else {
        preview
            .redactions
            .iter()
            .map(|(kind, count, distinct)| format!("{kind} {count}/{distinct}"))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    let full_summary = vec![
        Line::from(Span::styled(
            t(locale, UiText::RedactionPreview),
            theme::label(),
        )),
        fact(
            t(locale, UiText::Time),
            format!("{} .. {} UTC", preview.window_since, preview.window_until),
            width,
        ),
        fact(
            t(locale, UiText::Source),
            format!(
                "{} · {} {} · {} {}",
                preview.source,
                t(locale, UiText::Read),
                preview.lines_read,
                t(locale, UiText::Kept),
                preview.lines_kept
            ),
            width,
        ),
        fact(t(locale, UiText::Redactions), redactions, width),
        fact(
            t(locale, UiText::ReadErrors),
            preview.read_errors.to_string(),
            width,
        ),
        fact(
            t(locale, UiText::Output),
            preview.destination.clone(),
            width,
        ),
        privacy_line(&preview.token_map_path, width, locale),
    ];
    let summary_capacity = usize::from(area.height).saturating_sub(2);
    let mut lines = if summary_capacity >= full_summary.len() {
        full_summary
    } else {
        vec![
            Line::from(Span::styled(
                t(locale, UiText::RedactionPreview),
                theme::label(),
            )),
            fact(
                t(locale, UiText::Source),
                format!(
                    "{} · {} {} · {} {}",
                    preview.source,
                    t(locale, UiText::Read),
                    preview.lines_read,
                    t(locale, UiText::Kept),
                    preview.lines_kept
                ),
                width,
            ),
            fact(
                t(locale, UiText::Output),
                preview.destination.clone(),
                width,
            ),
            privacy_line(&preview.token_map_path, width, locale),
        ]
    };
    let sample_limit = summary_capacity.saturating_sub(lines.len()).min(4);
    for sample in preview.samples.iter().take(sample_limit) {
        lines.push(fact(t(locale, UiText::Sample), sample.clone(), width));
    }
    lines.extend([
        Line::from(""),
        Line::from(vec![
            choice(
                t(locale, UiText::Cancel),
                app.log_export.choice == ConfirmChoice::Cancel,
            ),
            Span::styled(t(locale, UiText::DefaultSuffix), theme::muted()),
            Span::raw("    "),
            choice(
                t(locale, UiText::WriteExport),
                app.log_export.choice == ConfirmChoice::Confirm,
            ),
        ]),
    ]);
    frame.render_widget(Paragraph::new(lines), area);
}

fn fact(label: &'static str, value: String, width: usize) -> Line<'static> {
    let prefix = format!("{label:<10}");
    let value_width = width.saturating_sub(unicode_width::UnicodeWidthStr::width(prefix.as_str()));
    Line::from(vec![
        Span::styled(prefix, theme::muted()),
        Span::raw(truncate_cells(&value, value_width)),
    ])
}

fn privacy_line(path: &str, width: usize, locale: Locale) -> Line<'static> {
    let text = if width < 60 {
        format!(
            "{} · {}  {path}",
            t(locale, UiText::DoNotShare),
            t(locale, UiText::PrivateMap)
        )
    } else {
        format!(
            "{}  {path} ({})",
            t(locale, UiText::PrivateMap),
            t(locale, UiText::DoNotShareLowercase)
        )
    };
    Line::from(Span::styled(
        truncate_cells(&text, width),
        theme::attention(),
    ))
}

fn option_line(label: &'static str, value: &'static str, selected: bool) -> Line<'static> {
    Line::from(vec![
        Span::styled(if selected { "› " } else { "  " }, theme::focus()),
        Span::styled(format!("{label:<10}"), theme::muted()),
        Span::styled(
            format!("‹ {value} ›"),
            if selected {
                theme::focus()
            } else {
                theme::normal()
            },
        ),
    ])
}

fn choice(label: &'static str, selected: bool) -> Span<'static> {
    Span::styled(
        if selected {
            format!("› [{label}]")
        } else {
            format!("  [{label}]")
        },
        if selected {
            theme::focus()
        } else {
            Style::default().add_modifier(Modifier::DIM)
        },
    )
}

fn window_label(seconds: i64, locale: Locale) -> &'static str {
    match seconds {
        3_600 => t(locale, UiText::Last1Hour),
        7_200 => t(locale, UiText::Last2Hours),
        21_600 => t(locale, UiText::Last6Hours),
        86_400 => t(locale, UiText::Last24HoursRecent),
        _ => t(locale, UiText::Custom),
    }
}
