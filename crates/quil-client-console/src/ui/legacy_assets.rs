use crate::i18n::UiText;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::{
    app::{
        App, ConfirmChoice, LegacyDestinationChoice, LegacyMigrationFileKind, LegacyMigrationStep,
    },
    domain::{
        format_quil_amount, LegacyAssetsSnapshot, LegacyMigrationDestination, LegacyMigrationPhase,
    },
    i18n::{t, Locale},
};

use super::{chrome, text::compact_middle, theme};

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let locale = app.locale();
    if app.legacy_migration.step != LegacyMigrationStep::Inactive {
        draw_migration(frame, app, locale);
        return;
    }
    let footer = if app.narrow_read_detail {
        t(locale, UiText::EscQBackToListCtrlCQuit)
    } else if app.viewport_width <= 69
        && app
            .legacy_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.identity_available && !snapshot.coins.is_empty())
    {
        t(
            locale,
            UiText::SelectCoinEnterDetailsRReloadEscQBackCtrlCQuit,
        )
    } else {
        mt(locale, MigrationText::ListFooter)
    };
    let Some(screen) = chrome::screen(frame, t(locale, UiText::LegacyAssets), app, footer) else {
        return;
    };

    let rows = Layout::vertical([Constraint::Length(4), Constraint::Min(4)]).split(screen.body);
    let content_area = rows[0].union(rows[1]);

    if app.legacy_in_flight {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::LoadingLegacyAssetsByPage))
                .alignment(Alignment::Center)
                .block(chrome::panel(t(locale, UiText::AssetStatus))),
            content_area,
        );
    } else if let Some(error) = &app.legacy_error {
        frame.render_widget(
            Paragraph::new(crate::i18n::localize_service_error(locale, error).into_owned())
                .style(Style::default().fg(Color::Red))
                .wrap(Wrap { trim: true })
                .block(chrome::panel(t(locale, UiText::LoadFailedTitle))),
            content_area,
        );
    } else if let Some(snapshot) = &app.legacy_snapshot {
        if app.narrow_read_detail {
            draw_detail(frame, app, snapshot, locale, content_area);
        } else {
            draw_snapshot(frame, app, snapshot, locale, rows[0], rows[1]);
        }
    } else {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::NoDataPressRToLoad))
                .alignment(Alignment::Center)
                .block(chrome::panel(t(locale, UiText::AssetStatus))),
            content_area,
        );
    }
}

#[derive(Clone, Copy)]
enum MigrationText {
    Title,
    ListFooter,
    IdentityTitle,
    IdentityHelp,
    DestinationTitle,
    DestinationLabel,
    CurrentNode,
    ExistingConfig,
    CreateNew,
    Path,
    PreflightTitle,
    ProgressTitle,
    FilesTitle,
    MaskedKey,
    Recipient,
    Amount,
    Fee,
    Batch,
    RecipientConfigDirectory,
    EstimatedNet,
    ConfirmMigration,
    ConfirmHint,
    Waiting,
    Unknown,
    Rejected,
    NeedKey,
    Complete,
    RecoveryConfirm,
    FileConfirm,
    FileHint,
    BackHint,
}

fn mt(locale: Locale, key: MigrationText) -> &'static str {
    use MigrationText::*;
    t(
        locale,
        match key {
            Title => UiText::LegacyMigrationTitle,
            ListFooter => UiText::LegacyMigrationListFooter,
            IdentityTitle => UiText::LegacyMigrationIdentityTitle,
            IdentityHelp => UiText::LegacyMigrationIdentityHelp,
            DestinationTitle => UiText::LegacyMigrationDestinationTitle,
            DestinationLabel => UiText::LegacyMigrationDestinationLabel,
            CurrentNode => UiText::LegacyMigrationCurrentNode,
            ExistingConfig => UiText::LegacyMigrationExistingConfig,
            CreateNew => UiText::LegacyMigrationCreateNew,
            Path => UiText::LegacyMigrationPath,
            PreflightTitle => UiText::LegacyMigrationPreflightTitle,
            ProgressTitle => UiText::LegacyMigrationProgressTitle,
            FilesTitle => UiText::LegacyMigrationFilesTitle,
            MaskedKey => UiText::LegacyMigrationMaskedKey,
            Recipient => UiText::LegacyMigrationRecipient,
            Amount => UiText::LegacyMigrationAmount,
            Fee => UiText::LegacyMigrationFee,
            Batch => UiText::LegacyMigrationBatch,
            RecipientConfigDirectory => UiText::LegacyMigrationRecipientConfigDirectory,
            EstimatedNet => UiText::LegacyMigrationEstimatedNet,
            ConfirmMigration => UiText::LegacyMigrationConfirmMigration,
            ConfirmHint => UiText::LegacyMigrationConfirmHint,
            Waiting => UiText::LegacyMigrationWaiting,
            Unknown => UiText::LegacyMigrationUnknown,
            Rejected => UiText::LegacyMigrationRejected,
            NeedKey => UiText::LegacyMigrationNeedKey,
            Complete => UiText::LegacyMigrationComplete,
            RecoveryConfirm => UiText::LegacyMigrationRecoveryConfirm,
            FileConfirm => UiText::LegacyMigrationFileConfirm,
            FileHint => UiText::LegacyMigrationFileHint,
            BackHint => UiText::LegacyMigrationBackHint,
        },
    )
}

fn draw_migration(frame: &mut Frame<'_>, app: &App, locale: Locale) {
    let footer = match app.legacy_migration.step {
        LegacyMigrationStep::Files
            if app
                .legacy_migration
                .status
                .as_ref()
                .is_some_and(|s| s.source_key_persisted) =>
        {
            mt(locale, MigrationText::FileHint)
        }
        LegacyMigrationStep::Preflight => mt(locale, MigrationText::ConfirmHint),
        _ => mt(locale, MigrationText::BackHint),
    };
    let Some(screen) = chrome::screen(frame, mt(locale, MigrationText::Title), app, footer) else {
        return;
    };
    if app.legacy_migration.viewer_kind.is_some() {
        frame.render_widget(
            Paragraph::new(app.legacy_migration.viewer.expose())
                .wrap(Wrap { trim: false })
                .block(chrome::panel(match app.legacy_migration.viewer_kind {
                    Some(LegacyMigrationFileKind::Config) => "config.yml",
                    _ => "keys.yml",
                })),
            screen.body,
        );
        return;
    }
    let migration = &app.legacy_migration;
    let mut lines = Vec::new();
    let title = match migration.step {
        LegacyMigrationStep::Identity => MigrationText::IdentityTitle,
        LegacyMigrationStep::Destination => MigrationText::DestinationTitle,
        LegacyMigrationStep::Preflight => MigrationText::PreflightTitle,
        LegacyMigrationStep::Progress => MigrationText::ProgressTitle,
        LegacyMigrationStep::Files => MigrationText::FilesTitle,
        LegacyMigrationStep::Inactive => MigrationText::Title,
    };
    match migration.step {
        LegacyMigrationStep::Identity => {
            lines.push(Line::from(mt(locale, MigrationText::IdentityHelp)));
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "{}: {}",
                mt(locale, MigrationText::MaskedKey),
                migration.secret.masked()
            )));
        }
        LegacyMigrationStep::Destination => {
            for (choice, label) in [
                (
                    LegacyDestinationChoice::CurrentNode,
                    mt(locale, MigrationText::CurrentNode),
                ),
                (
                    LegacyDestinationChoice::ExistingConfig,
                    mt(locale, MigrationText::ExistingConfig),
                ),
                (
                    LegacyDestinationChoice::CreateNew,
                    mt(locale, MigrationText::CreateNew),
                ),
            ] {
                lines.push(Line::from(format!(
                    "{} {label}",
                    if migration.destination == choice {
                        "›"
                    } else {
                        " "
                    }
                )));
            }
            if migration.destination == LegacyDestinationChoice::ExistingConfig {
                lines.push(Line::from(""));
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::Path),
                    migration.existing_path
                )));
            }
        }
        LegacyMigrationStep::Preflight => {
            if let Some(preflight) = &migration.preflight {
                let destination = match &preflight.destination {
                    LegacyMigrationDestination::CurrentNode => MigrationText::CurrentNode,
                    LegacyMigrationDestination::ExistingConfig(_) => MigrationText::ExistingConfig,
                    LegacyMigrationDestination::CreateNew => MigrationText::CreateNew,
                };
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::MaskedKey),
                    preflight.source.masked_key
                )));
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::DestinationLabel),
                    mt(locale, destination)
                )));
                let path_label = mt(locale, MigrationText::RecipientConfigDirectory);
                let path_budget = usize::from(screen.body.width)
                    .saturating_sub(unicode_width::UnicodeWidthStr::width(path_label) + 2);
                lines.push(Line::from(format!(
                    "{path_label}: {}",
                    compact_middle(
                        &preflight.recipient_config_dir.display().to_string(),
                        path_budget
                    )
                )));
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::Amount),
                    format_quil_amount(preflight.source_amount)
                )));
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::Fee),
                    format_quil_amount(preflight.fees.iter().sum())
                )));
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::EstimatedNet),
                    format_quil_amount(preflight.estimated_net_amount)
                )));
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::Recipient),
                    compact_middle(
                        &preflight.recipient_address,
                        usize::from(screen.body.width.saturating_sub(18))
                    )
                )));
                for warning in &preflight.warnings {
                    lines.push(Line::styled(
                        crate::i18n::localize_legacy_migration_warning(locale, warning),
                        theme::attention(),
                    ));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(format!(
                    "{}  {}",
                    if migration.confirmation == ConfirmChoice::Confirm {
                        "[x]"
                    } else {
                        "[ ]"
                    },
                    mt(locale, MigrationText::ConfirmMigration)
                )));
            }
        }
        LegacyMigrationStep::Progress => {
            if let Some(status) = &migration.status {
                let (batch, total) = match status.phase {
                    LegacyMigrationPhase::Migrating { batch, total }
                    | LegacyMigrationPhase::AwaitingChainConfirmation { batch, total } => {
                        (batch, total)
                    }
                    _ => (status.current_batch + 1, status.batches.len()),
                };
                lines.push(Line::from(format!(
                    "{}: {batch}/{total}",
                    mt(locale, MigrationText::Batch)
                )));
                if let Some(preflight) = &migration.preflight {
                    lines.push(Line::from(format!(
                        "{}: {}",
                        mt(locale, MigrationText::Amount),
                        format_quil_amount(preflight.source_amount)
                    )));
                }
                lines.push(Line::from(format!(
                    "{}: {}",
                    mt(locale, MigrationText::Recipient),
                    compact_middle(
                        &status.recipient_address,
                        usize::from(screen.body.width.saturating_sub(18))
                    )
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(match status.phase {
                    LegacyMigrationPhase::SubmissionUnknown => mt(locale, MigrationText::Unknown),
                    LegacyMigrationPhase::RecoverableError => mt(locale, MigrationText::Rejected),
                    LegacyMigrationPhase::AwaitingSourceKey => mt(locale, MigrationText::NeedKey),
                    LegacyMigrationPhase::Complete => mt(locale, MigrationText::Complete),
                    _ => mt(locale, MigrationText::Waiting),
                }));
                if matches!(status.phase, LegacyMigrationPhase::AwaitingSourceKey) {
                    lines.push(Line::from(format!(
                        "{}: {}",
                        mt(locale, MigrationText::MaskedKey),
                        migration.secret.masked()
                    )));
                }
                if migration.recovery_armed {
                    lines.push(Line::from(""));
                    lines.push(Line::styled(
                        mt(locale, MigrationText::RecoveryConfirm),
                        theme::attention(),
                    ));
                }
            }
        }
        LegacyMigrationStep::Files => {
            if let Some(status) = &migration.status {
                lines.push(Line::styled(
                    mt(locale, MigrationText::Complete),
                    theme::success(),
                ));
                lines.push(Line::from(""));
                lines.push(Line::from(compact_middle(
                    &status
                        .recipient_config_dir
                        .join("config.yml")
                        .display()
                        .to_string(),
                    usize::from(screen.body.width.saturating_sub(4)),
                )));
                lines.push(Line::from(compact_middle(
                    &status
                        .recipient_config_dir
                        .join("keys.yml")
                        .display()
                        .to_string(),
                    usize::from(screen.body.width.saturating_sub(4)),
                )));
                if migration.file_confirmation.is_some() {
                    lines.push(Line::from(""));
                    lines.push(Line::styled(
                        mt(locale, MigrationText::FileConfirm),
                        theme::attention(),
                    ));
                }
            }
        }
        LegacyMigrationStep::Inactive => {}
    }
    if migration.in_flight {
        lines.push(Line::from(""));
        lines.push(Line::styled(t(locale, UiText::Loading), theme::attention()));
    }
    if let Some(error) = &migration.error {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            crate::i18n::localize_legacy_migration_error(locale, error),
            theme::failure(),
        ));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(chrome::panel(mt(locale, title))),
        screen.body,
    );
}

fn draw_snapshot(
    frame: &mut Frame<'_>,
    app: &App,
    snapshot: &LegacyAssetsSnapshot,
    locale: Locale,
    summary_area: Rect,
    body_area: Rect,
) {
    if !snapshot.identity_available {
        frame.render_widget(
            Paragraph::new(t(
                locale,
                UiText::TheCurrentConfigurationHasNoLegacyEd448IdentitySoNoLegacyAssetsCanBeQueried,
            ))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(chrome::panel(t(locale, UiText::NotApplicable))),
            summary_area.union(body_area),
        );
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(t(locale, UiText::AvailablePadded), theme::muted()),
            Span::styled(
                format!(
                    "{} QUIL",
                    format_quil_amount(snapshot.unshielded_total_base_units)
                ),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(t(locale, UiText::Unshielded), theme::muted()),
            Span::raw(snapshot.unshielded_count.to_string()),
            Span::raw("    "),
            Span::styled(t(locale, UiText::ShieldedPadded), theme::muted()),
            Span::raw(snapshot.shielded_count.to_string()),
        ]))
        .alignment(Alignment::Center)
        .block(chrome::panel(t(locale, UiText::Summary))),
        summary_area,
    );

    if body_area.width >= 72 {
        let columns = Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(body_area);
        draw_list(frame, app, snapshot, locale, columns[0]);
        draw_detail(frame, app, snapshot, locale, columns[1]);
    } else {
        draw_list(frame, app, snapshot, locale, body_area);
    }
}

fn draw_list(
    frame: &mut Frame<'_>,
    app: &App,
    snapshot: &LegacyAssetsSnapshot,
    locale: Locale,
    area: Rect,
) {
    let items = snapshot.coins.iter().map(|coin| {
        let state = if coin.shielded {
            t(locale, UiText::Shielded)
        } else {
            t(locale, UiText::Available)
        };
        ListItem::new(Line::from(vec![
            Span::raw(format!("{}  ", compact_middle(&coin.address_hex, 24))),
            Span::styled(
                state,
                if coin.shielded {
                    theme::muted()
                } else {
                    theme::success()
                },
            ),
        ]))
    });
    let selected = (!snapshot.coins.is_empty()).then_some(app.legacy_index);
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(
        List::new(items)
            .block(chrome::panel("Coins"))
            .highlight_style(theme::selected())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
    if snapshot.coins.is_empty() {
        frame.render_widget(
            Paragraph::new(t(locale, UiText::NoLegacyCoinsFound)).alignment(Alignment::Center),
            area,
        );
    }
}

fn draw_detail(
    frame: &mut Frame<'_>,
    app: &App,
    snapshot: &LegacyAssetsSnapshot,
    locale: Locale,
    area: Rect,
) {
    let lines = snapshot.coins.get(app.legacy_index).map_or_else(
        || vec![Line::from(t(locale, UiText::NoCoins))],
        |coin| {
            vec![
                Line::from(format!(
                    "{}: {} QUIL",
                    t(locale, UiText::Amount),
                    format_quil_amount(coin.amount_base_units)
                )),
                Line::from(format!("{} base units", coin.amount_base_units)),
                Line::from(format!(
                    "{}: {}",
                    t(locale, UiText::Status),
                    if coin.shielded {
                        t(locale, UiText::Shielded)
                    } else {
                        t(locale, UiText::Available)
                    }
                )),
                Line::from(""),
                Line::from(t(locale, UiText::CoinAddressLabel)),
                Line::from(compact_middle(
                    &coin.address_hex,
                    usize::from(area.width.saturating_sub(2)),
                )),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(chrome::panel(t(locale, UiText::CoinDetails))),
        area,
    );
}
