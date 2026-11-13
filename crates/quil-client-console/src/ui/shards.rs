use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    allocation::allocation_lifecycle,
    app::{selected_shard_allocation_index, App, ShardFocus},
    domain::{
        HealthConditionSnapshot, HealthTarget, ProverAction, ShardInfoSnapshot, ShardSnapshot,
    },
    i18n::{allocation_lifecycle_status_label, format_allocation_action_hint, t, Locale},
};

use super::{
    chrome,
    detail::{render_fields, DetailField},
    text::{compact_middle, truncate_cells},
    theme,
};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.shards.focus == ShardFocus::FreeCores {
        t(locale, UiText::SelectCoreRRefreshEscQBackCtrlCQuit)
    } else if frame.area().width < 50 && app.narrow_read_detail {
        t(locale, UiText::PgUpPgDnScrollEscQBackCExit)
    } else if frame.area().width < 70 && app.narrow_read_detail {
        t(locale, UiText::PgUpPgDnScrollEscQListCtrlCExit)
    } else if app.viewport_width <= 69
        && app
            .shards
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.shards.is_empty())
    {
        t(
            locale,
            UiText::ScopeSelectAActionsEnterDetailsRRefreshEscQBackCtrlCExit,
        )
    } else if frame.area().width >= 70 {
        t(
            locale,
            UiText::SelectPgUpPgDnDetailsAActionsScopeRRefreshEscQBackCtrlCExit,
        )
    } else {
        t(locale, UiText::ScopeSelectRRefreshEscQBackCtrlCExit)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::ShardsRewards), app, footer) else {
        return;
    };

    if screen.body.width < 70 && app.narrow_read_detail {
        if let Some(snapshot) = &app.shards.snapshot {
            draw_detail(frame, app, snapshot, screen.body);
            return;
        }
    }

    let rows = catalog_rows(screen.body);
    frame.render_widget(
        Paragraph::new(scope_line(app)).block(chrome::panel(t(locale, UiText::QueryScope))),
        rows[0],
    );
    if let Some(snapshot) = &app.shards.snapshot {
        draw_summary(frame, snapshot, rows[1], locale);
    } else {
        frame.render_widget(Paragraph::new(""), rows[1]);
    }

    if app.shards.in_flight {
        frame.render_widget(
            centered(
                t(locale, UiText::LoadingShardDirectoryAndRewardEstimates),
                locale,
            ),
            rows[2],
        );
    } else if let Some(error) = &app.shards.error {
        frame.render_widget(
            Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .block(chrome::panel(t(locale, UiText::LoadFailed))),
            rows[2],
        );
    } else if let Some(snapshot) = &app.shards.snapshot {
        draw_snapshot(frame, app, snapshot, rows[2]);
    } else {
        frame.render_widget(
            centered(t(locale, UiText::NoDataPressRToLoad), locale),
            rows[2],
        );
    }
}

fn catalog_rows(body: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Min(5),
    ])
    .areas(body)
}

fn catalog_columns(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)]).areas(area)
}

struct DetailViewport {
    visible: usize,
    limit: usize,
    overflowing: bool,
}

impl DetailViewport {
    fn new(total: usize, area: Rect) -> Self {
        let height = usize::from(area.height.saturating_sub(2));
        let overflowing = total > height;
        let visible = height.saturating_sub(usize::from(overflowing));
        let limit = if visible > 0 {
            total.saturating_sub(visible)
        } else {
            0
        };
        Self {
            visible,
            limit,
            overflowing,
        }
    }
}

/// Returns the scroll limit and visible rows using the renderer's layout.
pub(crate) fn detail_scroll_bounds(app: &App) -> (usize, usize) {
    if app.page != crate::app::Page::Shards
        || app.viewport_width < 42
        || app.viewport_height < 12
        || app.shards.in_flight
        || app.shards.error.is_some()
    {
        return (0, 0);
    }
    let Some(shard) = app
        .shards
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.shards.get(app.shards.index))
    else {
        return (0, 0);
    };
    let body = Rect::new(0, 1, app.viewport_width, app.viewport_height - 2);
    let area = if app.viewport_width < 70 {
        if !app.narrow_read_detail {
            return (0, 0);
        }
        body
    } else {
        catalog_columns(catalog_rows(body)[2])[1]
    };
    let rows = detail_lines_locale(app, shard, area.width.saturating_sub(2), app.locale());
    let viewport = DetailViewport::new(rows.len(), area);
    (viewport.limit, viewport.visible)
}

fn draw_summary(frame: &mut Frame<'_>, snapshot: &ShardInfoSnapshot, area: Rect, locale: Locale) {
    let summary = &snapshot.summary;
    let total = summary.total.max(snapshot.shards.len());
    let estimates = format!(
        "{} {} ({}/{}) · 60m {} ({}/{}) · 24h {} ({}/{})",
        t(locale, UiText::Theory),
        aggregate_estimate(summary.theoretical_daily_quil.as_deref()),
        summary.theoretical_coverage,
        total,
        aggregate_estimate(summary.hour_daily_quil.as_deref()),
        summary.hour_coverage,
        total,
        aggregate_estimate(summary.day_daily_quil.as_deref()),
        summary.day_coverage,
        total,
    );
    let directory = snapshot
        .network_provers
        .map(|value| format!(" · Provers {value}"))
        .unwrap_or_default();
    let directory_state = if snapshot.directory_stale {
        format!(" · {}", t(locale, UiText::Stale))
    } else {
        String::new()
    };
    let mut lines = vec![
        Line::from(format!(
            "{} {}/{} · {} {} · Data Shards {} · {} {}",
            t(locale, UiText::Allocated),
            summary.allocated,
            total,
            t(locale, UiText::ActiveProvers),
            summary.active_provers,
            summary.data_shards,
            t(locale, UiText::UnknownRing),
            summary.ring_unknown,
        )),
        Line::from(vec![
            Span::raw(format!("{estimates}{directory}{directory_state} · ")),
            Span::styled("PomW Basis ", theme::muted()),
            Span::raw(if snapshot.pomw_basis == "0" {
                "—"
            } else {
                snapshot.pomw_basis.as_str()
            }),
        ]),
    ];
    if let Some(concentration) = summary
        .seat_concentration
        .as_ref()
        .filter(|concentration| concentration.at_risk > 0)
    {
        lines.push(Line::styled(
            truncate_cells(
                &crate::i18n::seat_concentration_warning(
                    locale,
                    concentration.at_risk,
                    concentration.reported,
                ),
                usize::from(area.width),
            ),
            theme::attention(),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme::muted())
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn aggregate_estimate(value: Option<&str>) -> String {
    value
        .map(|value| format!("{value} Q/d"))
        .unwrap_or_else(|| "—".into())
}

fn scope_line(app: &App) -> Line<'static> {
    let state = if app.shards.in_flight {
        t(app.locale(), UiText::Loading)
    } else {
        t(app.locale(), UiText::Switchable)
    };
    let mut spans = vec![
        Span::styled(t(app.locale(), UiText::Scope), theme::muted()),
        Span::styled(
            format!("‹ {} ›", app.shards.scope.localized_label(app.locale())),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("    {state}"), theme::muted()),
    ];
    if let Some(notice) = &app.shards.notice {
        spans.push(Span::styled(format!("    {notice}"), theme::attention()));
    }
    Line::from(spans)
}

fn centered(text: &'static str, locale: Locale) -> Paragraph<'static> {
    Paragraph::new(text)
        .alignment(Alignment::Center)
        .block(chrome::panel(t(locale, UiText::ShardDirectory)))
}

fn draw_snapshot(frame: &mut Frame<'_>, app: &App, snapshot: &ShardInfoSnapshot, area: Rect) {
    if snapshot.shards.is_empty() {
        frame.render_widget(
            Paragraph::new(if snapshot.include_all {
                t(app.locale(), UiText::TheNodeReturnedNoShards)
            } else {
                t(
                    app.locale(),
                    UiText::ThisNodeHasNoAllocatedShardsPressToViewAllShards,
                )
            })
            .alignment(Alignment::Center)
            .block(summary_block(snapshot, area.width, app.locale())),
            area,
        );
        return;
    }

    if area.width >= 70 {
        let columns = catalog_columns(area);
        draw_list(
            frame,
            app,
            snapshot,
            app.shards.index,
            columns[0],
            app.locale(),
            app.shards.focus == ShardFocus::Directory,
        );
        let show_core_panel = snapshot.include_all
            && (!idle_core_ids(app).is_empty() || app.prover_operations.has_pending());
        if show_core_panel {
            let [detail, cores] =
                Layout::vertical([Constraint::Percentage(62), Constraint::Percentage(38)])
                    .areas(columns[1]);
            draw_detail(frame, app, snapshot, detail);
            draw_idle_cores(frame, app, cores);
        } else {
            draw_detail(frame, app, snapshot, columns[1]);
        }
    } else if app.shards.focus == ShardFocus::FreeCores {
        draw_idle_cores(frame, app, area);
    } else {
        draw_list(
            frame,
            app,
            snapshot,
            app.shards.index,
            area,
            app.locale(),
            true,
        );
    }
}

fn draw_list(
    frame: &mut Frame<'_>,
    app: &App,
    snapshot: &ShardInfoSnapshot,
    index: usize,
    area: Rect,
    locale: Locale,
    focused: bool,
) {
    let items = snapshot.shards.iter().map(|shard| {
        let condition = shard_health_condition(app, shard);
        let health_marker = condition.map_or("", |condition| {
            super::health::severity_marker(&condition.severity)
        });
        let health_prefix = (!health_marker.is_empty())
            .then(|| format!("{health_marker} "))
            .unwrap_or_default();
        let marker = if shard.is_allocated {
            t(locale, UiText::Mine)
        } else {
            "    "
        };
        let ring = shard
            .ring
            .map(|value| {
                if shard.ring_estimated {
                    format!("~{value}")
                } else {
                    value.to_string()
                }
            })
            .unwrap_or_else(|| t(locale, UiText::Unknown).into());
        let reward = shard
            .theoretical_daily_quil
            .as_deref()
            .map(|value| format!("{value} Q/d"))
            .unwrap_or_else(|| t(locale, UiText::RewardUnknown).into());
        let text = if area.width < 62 {
            format!(
                "{health_prefix}{marker:<4} {}  R {ring}  {reward}",
                compact_middle(&shard.filter_hex, 12),
            )
        } else {
            format!(
                "{health_prefix}{marker:<4} {}  Ring {ring:<4}  Prover {:<4}  {reward}",
                compact_middle(&shard.filter_hex, 18),
                shard.active_provers,
            )
        };
        ListItem::new(text)
            .style(condition.map_or_else(theme::normal, super::health::condition_style))
    });
    let mut state = ListState::default().with_selected(Some(index));
    frame.render_stateful_widget(
        List::new(items)
            .block(
                summary_block(snapshot, area.width, locale).border_style(if focused {
                    theme::focus()
                } else {
                    theme::muted()
                }),
            )
            .highlight_style(Style::default().add_modifier(Modifier::BOLD))
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn draw_idle_cores(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let cores = idle_core_ids(app);
    let mut items = cores
        .iter()
        .map(|core_id| {
            ListItem::new(format!(
                "Core {core_id}  ·  {}",
                t(app.locale(), UiText::JoinShard)
            ))
        })
        .collect::<Vec<_>>();
    for operation in app
        .prover_operations
        .items
        .iter()
        .filter(|operation| operation.is_pending())
    {
        let action = match &operation.action {
            ProverAction::Join { .. } => "JOINING",
            ProverAction::Leave { .. } => "LEAVING",
            _ => "PENDING",
        };
        items.push(ListItem::new(Line::from(vec![
            Span::styled(action, theme::attention()),
            Span::raw(format!(
                "  {}  Core {}",
                compact_middle(&operation.filter_hex, 14),
                operation
                    .core_id
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "—".into())
            )),
        ])));
    }
    if items.is_empty() {
        items.push(ListItem::new(t(app.locale(), UiText::NoCoreBound)));
    }
    let selected = if cores.is_empty() {
        None
    } else {
        Some(app.shards.core_index.min(cores.len() - 1))
    };
    let mut state = ListState::default().with_selected(selected);
    let title = t(app.locale(), UiText::IdleCores);
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::focus_panel(
                title,
                app.shards.focus == ShardFocus::FreeCores,
            ))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn idle_core_ids(app: &App) -> Vec<u32> {
    let Some(snapshot) = app
        .snapshot
        .as_ref()
        .filter(|snapshot| snapshot.worker_info_error.is_none())
    else {
        return Vec::new();
    };
    let mut cores = snapshot
        .workers
        .iter()
        .filter(|worker| {
            worker.manually_managed
                && worker.filter_hex.is_empty()
                && !app.prover_operations.conflicts_core(worker.core_id)
        })
        .map(|worker| worker.core_id)
        .collect::<Vec<_>>();
    cores.sort_unstable();
    cores
}

fn summary_block(snapshot: &ShardInfoSnapshot, width: u16, locale: Locale) -> Block<'static> {
    let strategy = if snapshot.allocation_scoring_strategy.is_empty() {
        t(locale, UiText::NodeDefault)
    } else {
        snapshot.allocation_scoring_strategy.as_str()
    };
    chrome::panel(truncate_cells(
        &format!(
            "Frame {} · {} {} · {} {} · {} {} · {strategy}",
            snapshot.frame_number,
            snapshot.shards.len(),
            t(locale, UiText::ShardCountSuffix),
            t(locale, UiText::Difficulty),
            snapshot.difficulty,
            t(locale, UiText::StateLowercase),
            snapshot.world_state,
        ),
        usize::from(width.saturating_sub(4)),
    ))
}

fn draw_detail(frame: &mut Frame<'_>, app: &App, snapshot: &ShardInfoSnapshot, area: Rect) {
    let Some(shard) = snapshot.shards.get(app.shards.index) else {
        return;
    };
    let width = area.width.saturating_sub(2);
    let all_lines = detail_lines_locale(app, shard, width, app.locale());
    let total = all_lines.len();
    let viewport = DetailViewport::new(total, area);
    let offset = app.shards.detail_scroll.min(viewport.limit);
    let mut lines = all_lines
        .into_iter()
        .skip(offset)
        .take(viewport.visible)
        .collect::<Vec<_>>();
    if viewport.overflowing {
        let keys = if frame.area().width < 70 {
            "↑↓/PgUp/PgDn"
        } else {
            "PgUp/PgDn"
        };
        lines.push(Line::styled(
            truncate_cells(
                &format!(
                    "{keys} {}–{}/{} {}",
                    offset + 1,
                    offset + viewport.visible,
                    total,
                    t(app.locale(), UiText::Rows)
                ),
                usize::from(width),
            ),
            theme::muted(),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines).block(chrome::panel(t(app.locale(), UiText::ShardDetails))),
        area,
    );
}

#[cfg(test)]
fn detail_lines(shard: &ShardSnapshot, width: u16) -> Vec<Line<'static>> {
    detail_lines_locale(&App::default(), shard, width, Locale::TraditionalChinese)
}

fn detail_lines_locale(
    app: &App,
    shard: &ShardSnapshot,
    width: u16,
    locale: Locale,
) -> Vec<Line<'static>> {
    let ring = shard
        .ring
        .map(|value| {
            if shard.ring_estimated {
                format!("~{value}")
            } else {
                value.to_string()
            }
        })
        .unwrap_or_else(|| t(locale, UiText::UnknownNotConfirmedByNode).into());
    let theory = estimate(shard.theoretical_daily_quil.as_deref(), locale);
    let hour = estimate(shard.hour_daily_quil.as_deref(), locale);
    let day = estimate(shard.day_daily_quil.as_deref(), locale);
    let counts = shard
        .prover_counts
        .as_ref()
        .map(|value| {
            format!(
                "{} {} · {} {} · {} {} · {} {} · {} {} · {} {}",
                t(locale, UiText::Joining),
                value.joining,
                t(locale, UiText::Activating),
                value.activating,
                t(locale, UiText::ActiveStatus),
                value.active,
                t(locale, UiText::Paused),
                value.paused,
                t(locale, UiText::Leaving),
                value.leaving,
                t(locale, UiText::Departing),
                value.departing,
            )
        })
        .unwrap_or_else(|| t(locale, UiText::NodeReturnedNoStateDetails).into());
    let reason = shard.allocation_reason.as_deref().unwrap_or("—");
    let mut fields = vec![
        DetailField::new("Filter", shard.filter_hex.clone()),
        DetailField::new("Ring", ring),
        DetailField::new(
            t(locale, UiText::ActiveProvers),
            shard.active_provers.to_string(),
        ),
        DetailField::new(t(locale, UiText::ShardSize), shard.shard_size.clone()),
        DetailField::new("Data Shards", shard.data_shards.to_string()),
        DetailField::new(
            "Frame",
            format!(
                "{} {} · {} {}",
                t(locale, UiText::Local),
                shard.materialized_frame,
                t(locale, UiText::Latest),
                shard.latest_frame
            ),
        ),
    ];
    if let Some(condition) = shard_health_condition(app, shard) {
        fields.insert(
            1,
            DetailField::new(
                t(locale, UiText::Health),
                format!(
                    "{} {}",
                    super::health::severity_marker(&condition.severity),
                    condition.notice
                ),
            ),
        );
    }
    if app.loading {
        fields.push(DetailField::new(
            t(locale, UiText::LocalAllocationStatus),
            t(locale, UiText::LoadingNodeStatusMessage),
        ));
    } else if let Some(error) = app.error.as_deref() {
        fields.push(DetailField::new(
            t(locale, UiText::LocalAllocationStatus),
            crate::i18n::localize_service_error(locale, error).into_owned(),
        ));
    } else if let Some(snapshot) = app.snapshot.as_ref() {
        if let Some(allocation) =
            selected_shard_allocation_index(app).and_then(|index| snapshot.allocations.get(index))
        {
            if snapshot
                .epoch
                .as_ref()
                .is_some_and(|epoch| epoch.length_frames > 0)
            {
                let lifecycle = allocation_lifecycle(
                    allocation,
                    snapshot.last_received_frame,
                    snapshot.epoch.as_ref(),
                );
                fields.push(DetailField::new(
                    t(locale, UiText::LocalAllocationStatus),
                    allocation_lifecycle_status_label(locale, lifecycle.status),
                ));
                if let Some(next) = lifecycle.next_action.as_ref() {
                    fields.push(DetailField::new(
                        t(locale, UiText::NextAction),
                        format_allocation_action_hint(locale, next, true),
                    ));
                }
                if let Some(default) = lifecycle.default_action.as_ref() {
                    fields.push(DetailField::new(
                        t(locale, UiText::DefaultAction),
                        format_allocation_action_hint(locale, default, true),
                    ));
                }
            } else {
                fields.push(DetailField::new(
                    t(locale, UiText::LocalAllocationStatus),
                    t(locale, UiText::AllocationLifecycleUnavailable),
                ));
            }
        } else {
            fields.push(DetailField::new(
                t(locale, UiText::LocalAllocationStatus),
                t(locale, UiText::NoMatchingLocalAllocation),
            ));
        }
    } else {
        fields.push(DetailField::new(
            t(locale, UiText::LocalAllocationStatus),
            t(locale, UiText::NodeStatusNotLoadedRToRefresh),
        ));
    }
    fields.extend([
        DetailField::new(t(locale, UiText::TheoreticalDailyReward), theory),
        DetailField::new(t(locale, UiText::Last60Minutes), hour),
        DetailField::new(t(locale, UiText::Last24Hours), day),
        DetailField::new(t(locale, UiText::ProverStatus), counts),
        DetailField::new(t(locale, UiText::AllocationReason), reason),
    ]);
    render_fields(fields, width)
}

fn shard_health_condition<'a>(
    app: &'a App,
    shard: &ShardSnapshot,
) -> Option<&'a HealthConditionSnapshot> {
    if app.health.target != HealthTarget::SelfNode {
        return None;
    }
    let health = app.health.snapshot.as_ref()?;
    if !health.conditions_reported {
        return None;
    }
    health.highest_condition_for_shard(&shard.filter_hex)
}

fn estimate(value: Option<&str>, locale: Locale) -> String {
    value
        .map(|value| format!("{value} {}", t(locale, UiText::QUILDay)))
        .unwrap_or_else(|| t(locale, UiText::TemporarilyUnavailable).into())
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    use super::*;
    use crate::domain::{
        AllocationSnapshot, AllocationStatus, EpochSnapshot, SeatConcentrationSnapshot,
        ShardProverCountsSnapshot, ShardSummarySnapshot,
    };

    fn detail_fixture() -> ShardSnapshot {
        ShardSnapshot {
            filter_hex: "0123456789abcdef".repeat(32),
            active_provers: 9,
            ring: None,
            ring_estimated: false,
            official: false,
            shard_size: "1.25 GiB".into(),
            data_shards: 12,
            is_allocated: true,
            materialized_frame: 899,
            latest_frame: 900,
            allocation_reason: Some("manual👩🏽‍💻e\u{301} allocation reason ".repeat(10)),
            theoretical_daily_quil: Some("12.34".into()),
            hour_daily_quil: None,
            day_daily_quil: Some("56.78".into()),
            prover_counts: Some(ShardProverCountsSnapshot {
                joining: 1234567890,
                activating: 1234567891,
                active: 1234567892,
                paused: 1234567893,
                leaving: 1234567894,
                departing: 1234567895,
            }),
        }
    }

    fn scroll_app(width: u16, height: u16) -> App {
        let shard = detail_fixture();
        let mut app = App {
            page: crate::app::Page::Shards,
            locale: Locale::TraditionalChinese,
            viewport_width: width,
            viewport_height: height,
            shards: crate::app::ShardPageState {
                snapshot: Some(ShardInfoSnapshot {
                    include_all: true,
                    frame_number: 900,
                    difficulty: 42,
                    world_state: "1.0 GB".into(),
                    pomw_basis: "123456".into(),
                    allocation_scoring_strategy: "reward".into(),
                    directory_collected_at_unix_seconds: None,
                    directory_stale: false,
                    directory_error: None,
                    directory_invalid_rows: 0,
                    network_provers: None,
                    summary: ShardSummarySnapshot::default(),
                    shards: vec![shard.clone(), shard],
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        if width < 70 {
            crate::app::reduce_input(&mut app, crate::event::InputEvent::Enter);
            assert!(app.narrow_read_detail);
        }
        app
    }

    fn rendered_buffer(app: &App) -> ratatui::buffer::Buffer {
        let mut terminal =
            Terminal::new(TestBackend::new(app.viewport_width, app.viewport_height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    #[test]
    fn seat_concentration_warning_is_visible_once_at_narrow_width() {
        let mut app = scroll_app(60, 24);
        app.narrow_read_detail = false;
        app.shards
            .snapshot
            .as_mut()
            .unwrap()
            .summary
            .seat_concentration = Some(SeatConcentrationSnapshot {
            at_risk: 1,
            reported: 3,
        });

        let buffer = rendered_buffer(&app);
        let content = (0..app.viewport_height)
            .flat_map(|row| {
                (0..app.viewport_width)
                    .map(|column| buffer[(column, row)].symbol().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<String>();

        assert_eq!(content.matches("1/3").count(), 1, "{content}");
    }

    fn buffer_row(buffer: &ratatui::buffer::Buffer, x: u16, y: u16, width: u16) -> String {
        (x..x + width)
            .filter(|&column| column == x || buffer[(column, y)].symbol() != " ")
            .map(|column| buffer[(column, y)].symbol())
            .collect()
    }

    fn scroll_area(width: u16, height: u16) -> Rect {
        let body = Rect::new(0, 1, width, height - 2);
        if width < 70 {
            body
        } else {
            let content = Layout::vertical([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(5),
            ])
            .split(body)[2];
            Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)])
                .split(content)[1]
        }
    }

    #[test]
    fn shard_detail_scroll_pages_never_skip_rows_at_minimum_heights() {
        use crate::{app::reduce_input, event::InputEvent};
        for width in [42, 60, 70, 80, 120] {
            for height in [12, 13, 14, 15, 16, 24] {
                let mut app = scroll_app(width, height);
                let area = scroll_area(width, height);
                let rows = detail_lines(&detail_fixture(), area.width - 2);
                let visible = usize::from(area.height - 3);
                let limit = rows.len() - visible;
                let step = 5.min(visible.max(1));
                let mut seen = std::collections::HashSet::new();
                loop {
                    let start = app.shards.detail_scroll;
                    let buffer = rendered_buffer(&app);
                    for index in 0..visible {
                        let expected = rows[start + index]
                            .spans
                            .iter()
                            .map(|s| s.content.as_ref())
                            .collect::<String>();
                        let actual = buffer_row(
                            &buffer,
                            area.x + 1,
                            area.y + 1 + index as u16,
                            area.width - 2,
                        );
                        assert_eq!(
                            actual.replace(' ', ""),
                            expected.replace(' ', ""),
                            "{width}x{height}, row {}",
                            start + index
                        );
                        seen.insert(start + index);
                    }
                    reduce_input(&mut app, InputEvent::PageDown);
                    assert_eq!(
                        app.shards.detail_scroll,
                        (start + step).min(limit),
                        "{width}x{height}: visible {visible}"
                    );
                    if start == limit {
                        break;
                    }
                }
                assert_eq!(seen.len(), rows.len(), "{width}x{height}: skipped rows");
                seen.clear();
                loop {
                    let start = app.shards.detail_scroll;
                    for index in start..start + visible {
                        seen.insert(index);
                    }
                    reduce_input(&mut app, InputEvent::PageUp);
                    assert_eq!(
                        app.shards.detail_scroll,
                        start.saturating_sub(step),
                        "{width}x{height}"
                    );
                    if start == 0 {
                        break;
                    }
                }
                assert_eq!(
                    seen.len(),
                    rows.len(),
                    "{width}x{height}: reverse skipped rows"
                );
                assert!(app.take_shard_request().is_none());
            }
        }
    }

    #[test]
    fn shard_detail_scroll_reaches_every_row_at_24_lines_with_truthful_controls() {
        use crate::{app::reduce_input, event::InputEvent};
        for width in [42, 60, 70, 80, 120] {
            let mut app = scroll_app(width, 24);
            let area = scroll_area(width, 24);
            let rows = detail_lines(&detail_fixture(), area.width - 2);
            let visible = usize::from(area.height - 3);
            let limit = rows.len() - visible;
            let keys = if width < 70 {
                "↑↓/PgUp/PgDn"
            } else {
                "PgUp/PgDn"
            };
            let mut seen = std::collections::HashSet::new();
            let mut start = 0;
            loop {
                let buffer = rendered_buffer(&app);
                let status = buffer_row(&buffer, area.x + 1, area.bottom() - 2, area.width - 2);
                let expected_status =
                    format!("{keys} {}–{}/{} 行", start + 1, start + visible, rows.len());
                assert_eq!(
                    status.replace(' ', ""),
                    expected_status.replace(' ', ""),
                    "width {width}"
                );
                let footer = buffer_row(&buffer, 0, 23, width);
                assert!(footer.contains("PgUp/PgDn"), "{width}: {footer}");
                if width < 70 {
                    assert!(footer.contains("↑↓"));
                    if width < 50 {
                        assert!(footer.contains("Esc/Q返回"));
                    } else {
                        assert!(footer.contains("Esc/Q返回清單"));
                    }
                }
                for index in 0..visible {
                    let expected = rows[start + index]
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>();
                    let actual = buffer_row(
                        &buffer,
                        area.x + 1,
                        area.y + 1 + index as u16,
                        area.width - 2,
                    );
                    assert_eq!(
                        actual.replace(' ', ""),
                        expected.replace(' ', ""),
                        "{width}, row {}",
                        start + index
                    );
                    seen.insert(start + index);
                }
                assert!(app.shards.queued.is_none());
                assert!(!app.shards.in_flight);
                assert_eq!(app.shards.index, 0);
                if start == limit {
                    reduce_input(&mut app, InputEvent::PageDown);
                    assert_eq!(rendered_buffer(&app), buffer);
                    break;
                }
                reduce_input(&mut app, InputEvent::PageDown);
                start = (start + 5).min(limit);
            }
            assert_eq!(seen.len(), rows.len());
            while start > 0 {
                reduce_input(&mut app, InputEvent::PageUp);
                start = start.saturating_sub(5);
            }
            let top = rendered_buffer(&app);
            reduce_input(&mut app, InputEvent::PageUp);
            assert_eq!(rendered_buffer(&app), top);
            assert_eq!(top, rendered_buffer(&scroll_app(width, 24)));
        }
    }

    #[test]
    fn shard_detail_scroll_narrow_arrows_move_one_row_and_back_resets_without_requests() {
        use crate::{app::reduce_input, event::InputEvent};
        for width in [42, 60] {
            let mut app = scroll_app(width, 24);
            reduce_input(&mut app, InputEvent::Down);
            assert_eq!(app.shards.detail_scroll, 1);
            reduce_input(&mut app, InputEvent::PageDown);
            assert_eq!(app.shards.detail_scroll, 6);
            reduce_input(&mut app, InputEvent::Up);
            assert_eq!(app.shards.detail_scroll, 5);
            let limit = detail_scroll_bounds(&app).0;
            app.shards.detail_scroll = usize::MAX;
            reduce_input(&mut app, InputEvent::Up);
            assert_eq!(app.shards.detail_scroll, limit - 1);
            app.shards.detail_scroll = usize::MAX;
            reduce_input(&mut app, InputEvent::PageUp);
            assert_eq!(app.shards.detail_scroll, limit.saturating_sub(5));
            for _ in 0..limit + 2 {
                reduce_input(&mut app, InputEvent::Down);
            }
            assert_eq!(app.shards.detail_scroll, limit);
            assert_eq!(app.shards.index, 0);
            assert!(app.take_shard_request().is_none());
            reduce_input(&mut app, InputEvent::Back);
            assert!(!app.narrow_read_detail);
            assert_eq!(app.shards.detail_scroll, 0);
            assert_eq!(app.page, crate::app::Page::Shards);
            reduce_input(&mut app, InputEvent::PageDown);
            assert_eq!(app.shards.detail_scroll, 0);
            reduce_input(&mut app, InputEvent::Down);
            assert_eq!(app.shards.index, 1);
            app.shards.detail_scroll = 8;
            reduce_input(&mut app, InputEvent::Enter);
            assert_eq!(app.shards.detail_scroll, 0);
            assert!(app.narrow_read_detail);
            assert!(app.take_shard_request().is_none());
        }
    }

    #[test]
    fn shard_detail_scroll_wide_selection_resets_and_refresh_scope_resize_navigation_reset() {
        use crate::{app::reduce_input, event::InputEvent};
        for width in [70, 80, 120] {
            let mut app = scroll_app(width, 24);
            for event in [InputEvent::Down, InputEvent::Up] {
                app.shards.detail_scroll = 5;
                let previous = app.shards.index;
                reduce_input(&mut app, event);
                assert_ne!(app.shards.index, previous);
                assert_eq!(app.shards.detail_scroll, 0);
                assert!(app.take_shard_request().is_none());
            }
        }
        for event in [
            InputEvent::Refresh,
            InputEvent::Left,
            InputEvent::Right,
            InputEvent::Resize(42, 24),
            InputEvent::Back,
        ] {
            let mut app = scroll_app(80, 24);
            app.shards.detail_scroll = 5;
            let requests_expected = matches!(
                event,
                InputEvent::Refresh | InputEvent::Left | InputEvent::Right
            );
            reduce_input(&mut app, event);
            assert_eq!(app.shards.detail_scroll, 0);
            assert_eq!(app.take_shard_request().is_some(), requests_expected);
        }
        let mut app = scroll_app(60, 24);
        app.shards.detail_scroll = 5;
        reduce_input(&mut app, InputEvent::Resize(80, 24));
        assert_eq!(app.shards.detail_scroll, 0);
        assert!(!app.narrow_read_detail);
        app.shards.detail_scroll = 5;
        app.open(crate::app::Page::Help);
        assert_eq!(app.shards.detail_scroll, 0);
    }

    #[test]
    fn shard_detail_scroll_current_result_resets_and_stale_result_is_ignored() {
        use crate::{app::reduce_service, event::ServiceEvent};
        for success in [true, false] {
            let mut app = scroll_app(80, 24);
            let snapshot = app.shards.snapshot.clone().unwrap();
            app.shards.detail_scroll = 5;
            reduce_service(
                &mut app,
                ServiceEvent::ShardsFinished {
                    generation: 99,
                    result: Ok(snapshot.clone()),
                },
            );
            assert_eq!(app.shards.detail_scroll, 5);
            let result = if success {
                Ok(snapshot)
            } else {
                Err("offline".into())
            };
            let generation = app.shards.generation;
            reduce_service(
                &mut app,
                ServiceEvent::ShardsFinished { generation, result },
            );
            assert_eq!(app.shards.detail_scroll, 0);
            assert!(app.take_shard_request().is_none());
            if !success {
                assert_eq!(detail_scroll_bounds(&app).0, 0);
            }
        }
    }

    #[test]
    fn shard_detail_preserves_long_filter_and_prover_state_as_complete_fields() {
        let shard = detail_fixture();
        for width in [40, 58, 35, 53] {
            let mut fields = Vec::<(String, String)>::new();
            let lines = detail_lines(&shard, width);
            let prefix_width = lines[0].spans[0].content.width();
            for line in lines {
                let row = line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>();
                assert!(row.width() <= usize::from(width), "width {width}: {row:?}");
                assert!(!row.contains('…'), "field was truncated: {row:?}");
                if let Some(value) = row.strip_prefix(&" ".repeat(prefix_width)) {
                    fields
                        .last_mut()
                        .expect("continuation follows a key")
                        .1
                        .push_str(value);
                } else {
                    let (key, value) = row
                        .split_once(": ")
                        .expect("every field uses a fixed separator");
                    assert_eq!(key.width(), prefix_width - 2);
                    fields.push((key.trim_end().into(), value.into()));
                }
            }
            assert_eq!(fields, vec![
                ("Filter".into(), shard.filter_hex.clone()),
                ("Ring".into(), "未知（節點尚未確認）".into()),
                ("活躍 Prover".into(), "9".into()),
                ("分片大小".into(), "1.25 GiB".into()),
                ("Data Shards".into(), "12".into()),
                ("Frame".into(), "本機 899 · 最新 900".into()),
                ("本機分配狀態".into(), "尚未讀取節點狀態 · R 重新整理".into()),
                ("理論每日收益".into(), "12.34 QUIL/天".into()),
                ("近 60 分鐘".into(), "暫時無法使用".into()),
                ("近 24 小時".into(), "56.78 QUIL/天".into()),
                ("網路 Prover".into(), "加入 1234567890 · 啟用中 1234567891 · 活躍 1234567892 · 暫停 1234567893 · 離開 1234567894 · 退出 1234567895".into()),
                ("分配原因".into(), shard.allocation_reason.clone().unwrap()),
            ], "width {width}");
        }
    }

    #[test]
    fn shard_detail_shows_local_allocation_lifecycle_and_network_prover_state() {
        let mut shard = detail_fixture();
        shard.filter_hex = "11".repeat(32);
        let mut app = scroll_app(160, 40);
        app.shards.snapshot.as_mut().unwrap().shards = vec![shard];
        app.snapshot = Some(crate::domain::NodeSnapshot {
            last_received_frame: 150,
            epoch: Some(EpochSnapshot {
                current: 1,
                length_frames: 100,
            }),
            allocations: vec![AllocationSnapshot {
                filter_hex: "11".repeat(32),
                status: AllocationStatus::Leaving,
                leave_confirm_frame_number: 150,
                ..Default::default()
            }],
            ..Default::default()
        });

        let buffer = rendered_buffer(&app);
        let content = (0..app.viewport_height)
            .map(|row| buffer_row(&buffer, 0, row, app.viewport_width))
            .collect::<Vec<_>>()
            .join("")
            .replace(' ', "");

        assert!(content.contains("本機分配狀態:退出"), "{content}");
        assert!(
            content.contains("預設動作:退出·Frame200·剩餘Frame50"),
            "{content}"
        );
        assert!(content.contains("網路Prover:"), "{content}");

        let allocation = &mut app.snapshot.as_mut().unwrap().allocations[0];
        allocation.leave_confirm_frame_number = 0;
        allocation.leave_frame_number = 150;
        let buffer = rendered_buffer(&app);
        let content = (0..app.viewport_height)
            .map(|row| buffer_row(&buffer, 0, row, app.viewport_width))
            .collect::<Vec<_>>()
            .join("")
            .replace(' ', "");
        assert!(content.contains("本機分配狀態:離開"), "{content}");
        assert!(
            content.contains("下一個動作:拒絕/確認·Frame200·剩餘Frame50"),
            "{content}"
        );

        app.snapshot.as_mut().unwrap().epoch = None;
        let buffer = rendered_buffer(&app);
        let content = (0..app.viewport_height)
            .map(|row| buffer_row(&buffer, 0, row, app.viewport_width))
            .collect::<Vec<_>>()
            .join("")
            .replace(' ', "");
        assert!(
            content.contains("本機分配狀態:無法判斷分配週期；節點未回傳Epoch長度"),
            "{content}"
        );

        app.snapshot.as_mut().unwrap().allocations.clear();
        let buffer = rendered_buffer(&app);
        let content = (0..app.viewport_height)
            .map(|row| buffer_row(&buffer, 0, row, app.viewport_width))
            .collect::<Vec<_>>()
            .join("")
            .replace(' ', "");
        assert!(
            content.contains("本機分配狀態:找不到對應的本機分配記錄"),
            "{content}"
        );
    }

    #[test]
    fn shard_detail_aligns_real_buffer_columns_at_supported_screen_widths() {
        let shard = detail_fixture();
        let snapshot = ShardInfoSnapshot {
            include_all: true,
            frame_number: 900,
            difficulty: 42,
            world_state: "1.0 GB".into(),
            pomw_basis: "123456".into(),
            allocation_scoring_strategy: "reward".into(),
            directory_collected_at_unix_seconds: None,
            directory_stale: false,
            directory_error: None,
            directory_invalid_rows: 0,
            network_provers: None,
            summary: ShardSummarySnapshot::default(),
            shards: vec![shard.clone()],
        };
        for screen_width in [42, 60, 80, 120] {
            let body = Rect::new(0, 1, screen_width, 198);
            let area = if screen_width >= 78 {
                let rows = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Min(5),
                ])
                .split(body);
                Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)])
                    .split(rows[2])[1]
            } else {
                body
            };
            let app = App {
                page: crate::app::Page::Shards,
                locale: Locale::TraditionalChinese,
                viewport_width: screen_width,
                viewport_height: 200,
                narrow_read_detail: screen_width < 70,
                shards: crate::app::ShardPageState {
                    snapshot: Some(snapshot.clone()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut terminal = Terminal::new(TestBackend::new(screen_width, 200)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let rows = detail_lines(&shard, area.width - 2);
            let prefix_width = rows[0].spans[0].content.width() as u16;
            for (index, line) in rows.iter().enumerate() {
                let y = area.y + 1 + index as u16;
                let x = area.x + 1;
                let prefix = line.spans[0].content.as_ref();
                if prefix.trim().is_empty() {
                    for offset in 0..prefix_width {
                        assert_eq!(buffer[(x + offset, y)].symbol(), " ");
                    }
                } else {
                    assert_eq!(
                        buffer[(x + prefix_width - 2, y)].symbol(),
                        ":",
                        "{screen_width} at row {index}"
                    );
                    assert_eq!(buffer[(x + prefix_width - 1, y)].symbol(), " ");
                }
                let row = line
                    .spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>();
                let mut offset = 0;
                for grapheme in row.graphemes(true) {
                    assert_eq!(buffer[(x + offset, y)].symbol(), grapheme);
                    offset += grapheme.width() as u16;
                }
                assert_eq!(buffer[(area.x, y)].symbol(), "│");
                assert_eq!(buffer[(area.right() - 1, y)].symbol(), "│");
            }
            assert_eq!(
                buffer[(area.x + 1 + prefix_width + 1, area.y + 1)].symbol(),
                "1"
            );
        }
    }
}
