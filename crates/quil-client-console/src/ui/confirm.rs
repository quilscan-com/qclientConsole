use crate::i18n::UiText;
use ratatui::{
    layout::{Constraint, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::{
    app::{App, ConfirmChoice},
    domain::ProverAction,
    i18n::t,
};

use super::{chrome, text::truncate_cells, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    let footer = if app.operation_in_flight {
        t(locale, UiText::SubmittingCtrlCExit)
    } else {
        t(locale, UiText::SelectEnterExecuteEscQBackCtrlCExit)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::ConfirmOperationTitle), app, footer)
    else {
        return;
    };
    frame.render_widget(
        chrome::panel(t(locale, UiText::ConfirmOperation)),
        screen.body,
    );
    let inner = screen.body.inner(Margin::new(1, 1));
    let [summary, prompt] =
        Layout::vertical([Constraint::Min(4), Constraint::Length(2)]).areas(inner);
    let width = usize::from(summary.width);
    let lines = confirmation_lines(app)
        .into_iter()
        .map(|line| truncate_cells(&line, width))
        .map(Line::from)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), summary);

    if app.operation_in_flight {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::SubmittingPleaseWait)).style(theme::attention()),
            prompt,
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                choice(
                    t(locale, UiText::CancelDefault),
                    app.confirm_choice == ConfirmChoice::Cancel,
                ),
                Span::raw("    "),
                choice(
                    t(locale, UiText::ConfirmSubmission),
                    app.confirm_choice == ConfirmChoice::Confirm,
                ),
            ])),
            prompt,
        );
    }
}

fn confirmation_lines(app: &App) -> Vec<String> {
    if let Some(action) = app.pending_action.as_ref() {
        let mut lines = vec![
            format!("{}        {}", t(app.locale(), UiText::Operation), action.localized_label(app.locale())),
            format!("{}        {}", t(app.locale(), UiText::Impact), t(app.locale(), UiText::AfterSubmissionTheNodeFollowsProtocolProcessingStateChangesMayTakeSeveralFrames)),
        ];
        lines.extend(
            action
                .filters()
                .iter()
                .map(|filter| format!("Filter      {}", hex::encode(filter))),
        );
        if let ProverAction::Join {
            filters,
            worker_ids,
            ..
        } = action
        {
            if let Some(core_id) = worker_ids.first() {
                lines.push(format!("Core        {core_id}"));
            }
            let target = filters.first().and_then(|filter| {
                let filter_hex = hex::encode(filter);
                app.shards
                    .snapshot
                    .as_ref()?
                    .shards
                    .iter()
                    .find(|shard| shard.filter_hex == filter_hex)
            });
            if let Some(target) = target {
                let ring = target.ring.map_or_else(
                    || t(app.locale(), UiText::UnknownRing).to_string(),
                    |ring| {
                        let source = if target.ring_estimated {
                            t(app.locale(), UiText::EstimatedFromOfficialDirectory)
                        } else {
                            t(app.locale(), UiText::ConfirmedByNode)
                        };
                        format!(
                            "{}{ring} ({source})",
                            if target.ring_estimated { "~" } else { "" }
                        )
                    },
                );
                lines.push(format!("Ring        {ring}"));
                lines.push(format!(
                    "{}        {}",
                    t(app.locale(), UiText::TheoreticalDailyReward),
                    target
                        .theoretical_daily_quil
                        .as_ref()
                        .map(|reward| format!("{reward} QUIL/day"))
                        .unwrap_or_else(|| t(app.locale(), UiText::Unavailable).into())
                ));
            }
        }
        return lines;
    }
    if let Some(mutation) = app.pending_alias_mutation.as_ref() {
        let mut lines = vec![
            format!(
                "{}        {}",
                t(app.locale(), UiText::Operation),
                mutation.localized_label(app.locale())
            ),
            format!(
                "{}        {}",
                t(app.locale(), UiText::Alias),
                mutation.name()
            ),
        ];
        if let crate::domain::AliasMutation::Put {
            address_hex, kind, ..
        } = mutation
        {
            lines.push(format!(
                "{}        {address_hex}",
                t(app.locale(), UiText::Address)
            ));
            lines.push(format!(
                "{}        {}",
                t(app.locale(), UiText::Type),
                if kind.is_empty() {
                    t(app.locale(), UiText::NotSet)
                } else {
                    kind.as_str()
                }
            ));
        }
        lines.push(format!(
            "{}        {}",
            t(app.locale(), UiText::Impact),
            t(
                app.locale(),
                UiText::OnlyModifiesLocalAliasYmlForTheCurrentNodeConfiguration
            )
        ));
        return lines;
    }
    if let Some(mutation) = app.pending_key_mutation.as_ref() {
        let mut lines = vec![
            format!(
                "{}        {}",
                t(app.locale(), UiText::Operation),
                mutation.localized_label(app.locale())
            ),
            format!("{}    {}", t(app.locale(), UiText::KeyName), mutation.id()),
        ];
        if let crate::domain::KeyMutation::Create { key_type, .. } = mutation {
            let type_name = crate::domain::CREATABLE_KEY_TYPES
                .iter()
                .find_map(|&(value, name)| (value == *key_type).then_some(name))
                .unwrap_or(t(app.locale(), UiText::Unknown));
            lines.push(format!(
                "{}    {type_name}",
                t(app.locale(), UiText::KeyType)
            ));
        }
        lines.push(format!(
            "{}        {}",
            t(app.locale(), UiText::Impact),
            t(
                app.locale(),
                UiText::ModifiesTheLocalKeystoreOnlyPublicKeysAreShown
            )
        ));
        return lines;
    }
    vec![
        t(app.locale(), UiText::NoOperationIsAwaitingConfirmation).into(),
        t(app.locale(), UiText::NoWriteOperationWillRun).into(),
    ]
}

fn choice(label: &'static str, selected: bool) -> Span<'static> {
    Span::styled(
        if selected {
            format!("› [ {label} ]")
        } else {
            format!("  [ {label} ]")
        },
        if selected {
            theme::focus()
        } else {
            Style::default().add_modifier(Modifier::DIM)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ShardPageState;
    use crate::domain::{ShardInfoSnapshot, ShardSnapshot, ShardSummarySnapshot};

    #[test]
    fn join_confirmation_names_core_ring_source_and_reward() {
        let filter = vec![0x11; 35];
        let filter_hex = hex::encode(&filter);
        let snapshot = ShardInfoSnapshot {
            include_all: true,
            frame_number: 1,
            difficulty: 0,
            world_state: String::new(),
            pomw_basis: String::new(),
            allocation_scoring_strategy: String::new(),
            directory_collected_at_unix_seconds: Some(1),
            directory_stale: false,
            directory_error: None,
            directory_invalid_rows: 0,
            network_provers: None,
            summary: ShardSummarySnapshot::default(),
            shards: vec![ShardSnapshot {
                filter_hex,
                active_provers: 16,
                ring: Some(2),
                ring_estimated: true,
                official: true,
                shard_size: "1.0 GB".into(),
                data_shards: 1,
                is_allocated: false,
                materialized_frame: 0,
                latest_frame: 1,
                allocation_reason: None,
                theoretical_daily_quil: Some("11.73".into()),
                hour_daily_quil: None,
                day_daily_quil: None,
                prover_counts: None,
            }],
        };
        let app = App {
            pending_action: Some(ProverAction::Join {
                filters: vec![filter],
                delegate: Vec::new(),
                worker_ids: vec![3],
            }),
            shards: ShardPageState {
                snapshot: Some(snapshot),
                ..Default::default()
            },
            ..Default::default()
        };

        let lines = confirmation_lines(&app).join("\n");

        assert!(lines.contains("Core        3"));
        assert!(lines.contains("Ring        ~2 (estimated from official directory)"));
        assert!(lines.contains("11.73 QUIL/day"));
    }
}
