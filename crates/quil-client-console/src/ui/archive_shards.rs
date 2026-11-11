use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::App,
    domain::{ArchiveConsensus, ArchiveEndpointSnapshot, ArchiveShardSnapshot},
    i18n::{t, Locale},
};

use super::{
    chrome::{self, Breakpoint},
    text::truncate_cells,
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let breakpoint = Breakpoint::from_width(frame.area().width);
    let footer = if breakpoint == Breakpoint::Narrow && app.archive_shards.detail {
        t(locale, UiText::EscQListRRefreshCExit)
    } else if breakpoint == Breakpoint::Narrow {
        t(locale, UiText::SelectEnterDetailsRRefreshEscQBackCExit)
    } else {
        t(locale, UiText::SelectRRecheckEscQBackCtrlCExit)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::ArchiveConsistency), app, footer)
    else {
        return;
    };

    let [summary, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(screen.body);

    if app.archive_shards.in_flight {
        frame.render_widget(
            state_message(
                t(locale, UiText::CheckingArchiveShardDirectories),
                theme::title(),
                locale,
            ),
            content,
        );
    } else if let Some(error) = &app.archive_shards.error {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(t(locale, UiText::CheckFailed), error_style())),
                Line::from(crate::i18n::localize_service_error(locale, error).into_owned()),
            ])
            .block(chrome::panel(t(locale, UiText::ServiceError))),
            content,
        );
    } else if let Some(snapshot) = &app.archive_shards.snapshot {
        frame.render_widget(summary_line(snapshot, locale), summary);
        draw_snapshot(
            frame,
            snapshot,
            app.archive_shards.index,
            app.archive_shards.detail,
            screen.breakpoint,
            content,
            locale,
        );
    } else {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::NotCheckedYetPressRToStart))
                .block(chrome::panel(t(locale, UiText::EndpointDirectory))),
            content,
        );
    }
}

fn state_message(text: &'static str, style: Style, locale: Locale) -> Paragraph<'static> {
    Paragraph::new(Span::styled(text, style))
        .block(chrome::panel(t(locale, UiText::EndpointDirectory)))
}

fn summary_line(snapshot: &ArchiveShardSnapshot, locale: Locale) -> Paragraph<'static> {
    let label = consensus_label(snapshot, locale);
    Paragraph::new(Line::from(vec![
        Span::styled(t(locale, UiText::Result), theme::muted()),
        Span::styled(label, consensus_style(snapshot.summary)),
        Span::raw(format!(
            "    {} {}/{} {}",
            t(locale, UiText::Answered),
            snapshot.answered,
            snapshot.total,
            t(locale, UiText::Endpoints)
        )),
    ]))
}

fn draw_snapshot(
    frame: &mut Frame<'_>,
    snapshot: &ArchiveShardSnapshot,
    index: usize,
    detail_open: bool,
    breakpoint: Breakpoint,
    area: Rect,
    locale: Locale,
) {
    if snapshot.archives.is_empty() {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::TheNodeReturnedNoArchiveEndpoints))
                .block(chrome::panel(t(locale, UiText::EndpointDirectory))),
            area,
        );
        return;
    }

    let selected = index.min(snapshot.archives.len().saturating_sub(1));
    if breakpoint != Breakpoint::Narrow {
        let columns = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(area);
        draw_list(frame, snapshot, selected, columns[0], locale);
        draw_detail(frame, &snapshot.archives[selected], columns[1], locale);
    } else if detail_open {
        draw_detail(frame, &snapshot.archives[selected], area, locale);
    } else {
        draw_list(frame, snapshot, selected, area, locale);
    }
}

fn draw_list(
    frame: &mut Frame<'_>,
    snapshot: &ArchiveShardSnapshot,
    index: usize,
    area: Rect,
    locale: Locale,
) {
    const COUNT_WIDTH: usize = 4;
    let preferred_endpoint_width = if area.width >= 54 {
        22
    } else if area.width >= 40 {
        13
    } else {
        9
    };
    let digest_width = if area.width >= 54 { 10 } else { 6 };
    let items = snapshot.archives.iter().map(|archive| {
        let status = status_label(&archive.status, locale);
        let fixed_width = 2 + COUNT_WIDTH + 1 + COUNT_WIDTH + 1 + digest_width + 2 + status.width();
        let content_width = usize::from(area.width.saturating_sub(2));
        let endpoint_width = preferred_endpoint_width
            .min(content_width.saturating_sub(2).saturating_sub(fixed_width));
        let prefix = format!(
            "{}  {} {} {}  ",
            fit_cells(&archive.endpoint, endpoint_width),
            row_count(archive.shard_count, COUNT_WIDTH),
            row_count(archive.frozen_count, COUNT_WIDTH),
            fit_cells(archive.digest.as_deref().unwrap_or("—"), digest_width),
        );
        ListItem::new(Line::from(vec![
            Span::raw(prefix),
            Span::styled(status, status_style(&archive.status)),
        ]))
    });
    let mut state = ListState::default().with_selected(Some(index));
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel(t(
                locale,
                UiText::EndpointShardsFrozenDigestStatus,
            )))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_detail(
    frame: &mut Frame<'_>,
    archive: &ArchiveEndpointSnapshot,
    area: Rect,
    locale: Locale,
) {
    let content_width = usize::from(area.width.saturating_sub(2));
    let content_height = usize::from(area.height.saturating_sub(2));
    frame.render_widget(
        Paragraph::new(detail_lines(archive, content_width, content_height, locale))
            .block(chrome::panel(t(locale, UiText::EndpointDetails))),
        area,
    );
}

fn detail_lines(
    archive: &ArchiveEndpointSnapshot,
    width: usize,
    height: usize,
    locale: Locale,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(t(locale, UiText::Endpoint), theme::muted())),
        Line::from(truncate_cells(&archive.endpoint, width)),
        Line::from(vec![
            Span::styled(fit_cells(t(locale, UiText::Status), 8), theme::muted()),
            Span::styled(
                status_label(&archive.status, locale),
                status_style(&archive.status),
            ),
        ]),
        detail_value(
            t(locale, UiText::Shards),
            optional_count(archive.shard_count),
            width,
        ),
        detail_value(
            t(locale, UiText::Frozen),
            optional_count(archive.frozen_count),
            width,
        ),
        detail_value(
            t(locale, UiText::Digest),
            archive.digest.clone().unwrap_or_else(|| "—".into()),
            width,
        ),
    ];

    if archive.status == "differs" || archive.status == "no majority" {
        let candidates = filter_detail_candidates(archive, width, locale);
        let available = height.saturating_sub(lines.len());
        if candidates.len() <= available {
            lines.extend(candidates.into_iter().map(|(text, _)| Line::from(text)));
        } else if available > 0 {
            let visible = available.saturating_sub(1);
            let shown_filters = candidates
                .iter()
                .take(visible)
                .filter(|(_, is_filter)| *is_filter)
                .count();
            let hidden_filters = archive
                .extra_filters
                .len()
                .saturating_add(archive.missing_filters.len())
                .saturating_sub(shown_filters);
            lines.extend(
                candidates
                    .into_iter()
                    .take(visible)
                    .map(|(text, _)| Line::from(text)),
            );
            lines.push(Line::from(Span::styled(
                truncate_cells(
                    &format!(
                        "{} {hidden_filters} {}",
                        t(locale, UiText::Another),
                        t(locale, UiText::ItemsHidden)
                    ),
                    width,
                ),
                warning_style(),
            )));
        }
    }
    if let Some(error) = &archive.error {
        if lines.len() < height {
            let label = fit_cells(t(locale, UiText::Reason), 8);
            let value = truncate_cells(error, width.saturating_sub(label.width()));
            lines.push(Line::from(vec![
                Span::styled(label, theme::muted()),
                Span::styled(value, error_style()),
            ]));
        }
    }
    lines
}

fn filter_detail_candidates(
    archive: &ArchiveEndpointSnapshot,
    width: usize,
    locale: Locale,
) -> Vec<(String, bool)> {
    let mut candidates = Vec::new();
    push_first_filter(
        &mut candidates,
        t(locale, UiText::Extra),
        &archive.extra_filters,
        width,
        locale,
    );
    push_first_filter(
        &mut candidates,
        t(locale, UiText::Missing),
        &archive.missing_filters,
        width,
        locale,
    );
    for (index, filter) in archive.extra_filters.iter().enumerate().skip(1) {
        candidates.push((
            numbered_filter_line(
                t(locale, UiText::Extra),
                index,
                archive.extra_filters.len(),
                filter,
                width,
            ),
            true,
        ));
    }
    for (index, filter) in archive.missing_filters.iter().enumerate().skip(1) {
        candidates.push((
            numbered_filter_line(
                t(locale, UiText::Missing),
                index,
                archive.missing_filters.len(),
                filter,
                width,
            ),
            true,
        ));
    }
    candidates
}

fn push_first_filter(
    candidates: &mut Vec<(String, bool)>,
    label: &str,
    filters: &[String],
    width: usize,
    locale: Locale,
) {
    if let Some(filter) = filters.first() {
        candidates.push((
            numbered_filter_line(label, 0, filters.len(), filter, width),
            true,
        ));
    } else {
        candidates.push((
            truncate_cells(&format!("{label} 0  {}", t(locale, UiText::None)), width),
            false,
        ));
    }
}

fn numbered_filter_line(
    label: &str,
    index: usize,
    total: usize,
    filter: &str,
    width: usize,
) -> String {
    let prefix = format!("{label} {}/{}  ", index + 1, total);
    format!(
        "{prefix}{}",
        truncate_cells(filter, width.saturating_sub(prefix.width()))
    )
}

fn detail_value(label: &'static str, value: String, width: usize) -> Line<'static> {
    let label = fit_cells(label, 8);
    let value = truncate_cells(&value, width.saturating_sub(label.width()));
    Line::from(vec![Span::styled(label, theme::muted()), Span::raw(value)])
}

fn optional_count(value: Option<usize>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "—".into())
}

fn row_count(value: Option<usize>, width: usize) -> String {
    let value = value.map(bounded_count).unwrap_or_else(|| "—".to_owned());
    fit_cells_right(&value, width)
}

fn bounded_count(value: usize) -> String {
    let value = value as u128;
    match value {
        0..=999 => value.to_string(),
        1_000..=999_999 => format!("{}k", value / 1_000),
        1_000_000..=999_999_999 => format!("{}M", value / 1_000_000),
        1_000_000_000..=999_999_999_999 => format!("{}B", value / 1_000_000_000),
        1_000_000_000_000..=999_999_999_999_999 => {
            format!("{}T", value / 1_000_000_000_000)
        }
        _ => "999+".into(),
    }
}

fn fit_cells(value: &str, width: usize) -> String {
    let mut value = truncate_cells(value, width);
    value.extend(std::iter::repeat_n(
        ' ',
        width.saturating_sub(value.width()),
    ));
    value
}

fn fit_cells_right(value: &str, width: usize) -> String {
    let value = truncate_cells(value, width);
    let padding = width.saturating_sub(value.width());
    format!("{}{value}", " ".repeat(padding))
}

fn consensus_label(snapshot: &ArchiveShardSnapshot, locale: Locale) -> String {
    if snapshot.summary == ArchiveConsensus::NoAnswers {
        return t(locale, UiText::AllFailed).into();
    }

    let consensus = match snapshot.summary {
        ArchiveConsensus::AllAgree => {
            if snapshot.answered < snapshot.total {
                t(locale, UiText::SuccessfulEndpointsAgree)
            } else {
                t(locale, UiText::AllAgree)
            }
        }
        ArchiveConsensus::Majority => t(locale, UiText::DifferencesFound),
        ArchiveConsensus::NoMajority => t(locale, UiText::NoClearMajority),
        ArchiveConsensus::NoAnswers => unreachable!(),
    };
    if snapshot.answered < snapshot.total {
        format!("{consensus} · {}", t(locale, UiText::SomeResponsesFailed))
    } else {
        consensus.into()
    }
}

fn status_label(status: &str, locale: Locale) -> &'static str {
    match status {
        "agrees" => t(locale, UiText::Agrees),
        "differs" => t(locale, UiText::Differs),
        "no majority" => t(locale, UiText::NoMajority),
        "error" => t(locale, UiText::Failed),
        _ => t(locale, UiText::Unknown),
    }
}

fn consensus_style(consensus: ArchiveConsensus) -> Style {
    match consensus {
        ArchiveConsensus::AllAgree => theme::success(),
        ArchiveConsensus::Majority => warning_style(),
        ArchiveConsensus::NoMajority => difference_style(),
        ArchiveConsensus::NoAnswers => error_style(),
    }
}

fn status_style(status: &str) -> Style {
    match status {
        "agrees" => theme::success(),
        "differs" => warning_style(),
        "no majority" => difference_style(),
        "error" => error_style(),
        _ => theme::muted(),
    }
}

fn error_style() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

fn warning_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn difference_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_filter_detail_uses_the_selected_language() {
        let archive = ArchiveEndpointSnapshot {
            endpoint: "archive".into(),
            shard_count: None,
            frozen_count: None,
            digest: None,
            status: "differs".into(),
            extra_filters: vec!["extra-first".into(), "extra-second".into()],
            missing_filters: vec!["missing-first".into(), "missing-second".into()],
            error: None,
        };
        for locale in [
            Locale::English,
            Locale::TraditionalChinese,
            Locale::Japanese,
            Locale::French,
        ] {
            let candidates = filter_detail_candidates(&archive, 100, locale);
            for (key, filter) in [
                (UiText::Extra, "extra-second"),
                (UiText::Missing, "missing-second"),
            ] {
                let row = candidates
                    .iter()
                    .find(|(row, _)| row.contains(filter))
                    .unwrap();
                assert!(row.0.starts_with(t(locale, key)), "{locale:?}: {}", row.0);
            }
        }
    }
}
