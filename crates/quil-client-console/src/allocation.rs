//! Console-owned allocation lifecycle calculations.
//!
//! Effective status and confirm windows follow the qclient protocol mirror and
//! derive their epoch and renew boundaries from the observed frame. The RPC
//! epoch supplies the epoch length only; its current-epoch field may lag or lead
//! the received frame and never changes lifecycle timing. When epoch length
//! telemetry is absent or zero, this module does not guess the mainnet 720-frame
//! value: testnets may use 60 frames, so the lifecycle is unknown and exposes
//! no actions. The raw RPC status remains available on [`AllocationSnapshot`].

use crate::domain::{AllocationSnapshot, AllocationStatus, EpochSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationLifecycleStatus {
    Unknown,
    Joining,
    Activating,
    Active,
    Paused,
    Leaving,
    Departing,
    Rejected,
    Kicked,
    Historic,
    ExpiredJoining,
    ExpiredLeaving,
    ExpiredEpoch,
}

impl AllocationLifecycleStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Joining => "joining",
            Self::Activating => "activating",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Leaving => "leaving",
            Self::Departing => "departing",
            Self::Rejected => "rejected",
            Self::Kicked => "kicked",
            Self::Historic => "historic",
            Self::ExpiredJoining => "expiredJoin",
            Self::ExpiredLeaving => "expiredLeave",
            Self::ExpiredEpoch => "re-confirm!",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationAction {
    Reject,
    Confirm,
    Pause,
    Resume,
    Leave,
    Renew,
    Activate,
    Depart,
    Expire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTiming {
    Now,
    At { frame: u64, frames_remaining: u64 },
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionHint {
    pub actions: Vec<AllocationAction>,
    pub timing: ActionTiming,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocationLifecycle {
    pub status: AllocationLifecycleStatus,
    pub label: &'static str,
    pub next_action: Option<ActionHint>,
    pub default_action: Option<ActionHint>,
}

impl AllocationLifecycle {
    pub fn new(
        status: AllocationLifecycleStatus,
        next_action: Option<ActionHint>,
        default_action: Option<ActionHint>,
    ) -> Self {
        Self {
            status,
            label: status.label(),
            next_action,
            default_action,
        }
    }
}

/// Calculate the allocation's effective epoch-aligned lifecycle.
///
/// `last_received_frame` must be [`crate::domain::NodeSnapshot::last_received_frame`].
/// Do not pass `last_global_head_frame`: qclient lifecycle semantics use the
/// node's last received frame, while the global head is a different clock.
pub fn allocation_lifecycle(
    allocation: &AllocationSnapshot,
    last_received_frame: u64,
    epoch: Option<&EpochSnapshot>,
) -> AllocationLifecycle {
    let Some(epoch) = epoch.filter(|epoch| epoch.length_frames > 0) else {
        return AllocationLifecycle::new(AllocationLifecycleStatus::Unknown, None, None);
    };

    let current_epoch = last_received_frame / epoch.length_frames;
    let status = effective_status(allocation, current_epoch, epoch.length_frames);
    let (next_action, default_action) = action_hints(
        allocation,
        status,
        last_received_frame,
        current_epoch,
        epoch,
    );
    AllocationLifecycle::new(status, next_action, default_action)
}

fn effective_status(
    allocation: &AllocationSnapshot,
    current_epoch: u64,
    epoch_length: u64,
) -> AllocationLifecycleStatus {
    match allocation.status {
        AllocationStatus::Unknown => AllocationLifecycleStatus::Unknown,
        AllocationStatus::Joining => {
            if proposal_expired(allocation.join_frame_number, current_epoch, epoch_length) {
                AllocationLifecycleStatus::ExpiredJoining
            } else {
                AllocationLifecycleStatus::Joining
            }
        }
        AllocationStatus::Active => {
            if allocation.filter_hex.is_empty() {
                return AllocationLifecycleStatus::Active;
            }
            if allocation.join_confirm_frame_number > 0
                && current_epoch
                    < effective_epoch(allocation.join_confirm_frame_number, epoch_length)
            {
                return AllocationLifecycleStatus::Activating;
            }
            if allocation.epoch >= current_epoch {
                AllocationLifecycleStatus::Active
            } else {
                AllocationLifecycleStatus::ExpiredEpoch
            }
        }
        AllocationStatus::Paused => AllocationLifecycleStatus::Paused,
        AllocationStatus::Leaving => {
            if allocation.leave_confirm_frame_number > 0 {
                if current_epoch
                    < effective_epoch(allocation.leave_confirm_frame_number, epoch_length)
                {
                    AllocationLifecycleStatus::Departing
                } else {
                    AllocationLifecycleStatus::ExpiredLeaving
                }
            } else if proposal_expired(allocation.leave_frame_number, current_epoch, epoch_length) {
                AllocationLifecycleStatus::ExpiredLeaving
            } else {
                AllocationLifecycleStatus::Leaving
            }
        }
        AllocationStatus::Rejected => AllocationLifecycleStatus::Rejected,
        AllocationStatus::Kicked => AllocationLifecycleStatus::Kicked,
        AllocationStatus::Historic => AllocationLifecycleStatus::Historic,
    }
}

fn proposal_expired(proposal_frame: u64, current_epoch: u64, epoch_length: u64) -> bool {
    proposal_frame > 0 && current_epoch > effective_epoch(proposal_frame, epoch_length)
}

fn effective_epoch(frame: u64, epoch_length: u64) -> u64 {
    (frame / epoch_length).saturating_add(1)
}

fn epoch_start(epoch: u64, epoch_length: u64) -> u64 {
    epoch.saturating_mul(epoch_length)
}

fn next_boundary(current_epoch: u64, epoch_length: u64) -> u64 {
    epoch_start(current_epoch.saturating_add(1), epoch_length)
}

fn boundary_after(frame: u64, epoch_length: u64) -> u64 {
    epoch_start(effective_epoch(frame, epoch_length), epoch_length)
}

fn action_hints(
    allocation: &AllocationSnapshot,
    status: AllocationLifecycleStatus,
    last_received_frame: u64,
    current_epoch: u64,
    epoch: &EpochSnapshot,
) -> (Option<ActionHint>, Option<ActionHint>) {
    match status {
        AllocationLifecycleStatus::Joining if allocation.status == AllocationStatus::Joining => {
            confirm_hints(
                allocation.join_frame_number,
                last_received_frame,
                current_epoch,
                epoch.length_frames,
            )
        }
        AllocationLifecycleStatus::Leaving if allocation.leave_confirm_frame_number == 0 => {
            confirm_hints(
                allocation.leave_frame_number,
                last_received_frame,
                current_epoch,
                epoch.length_frames,
            )
        }
        AllocationLifecycleStatus::Departing => (
            None,
            Some(at(
                &[AllocationAction::Depart],
                boundary_after(allocation.leave_confirm_frame_number, epoch.length_frames),
                last_received_frame,
            )),
        ),
        AllocationLifecycleStatus::Activating => (
            Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
            Some(at(
                &[AllocationAction::Activate],
                boundary_after(allocation.join_confirm_frame_number, epoch.length_frames),
                last_received_frame,
            )),
        ),
        AllocationLifecycleStatus::Active | AllocationLifecycleStatus::ExpiredEpoch => {
            let default_action = (!allocation.filter_hex.is_empty()).then(|| {
                at(
                    &[AllocationAction::Renew],
                    next_boundary(current_epoch, epoch.length_frames),
                    last_received_frame,
                )
            });
            (
                Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                default_action,
            )
        }
        AllocationLifecycleStatus::Paused => (
            Some(now(&[AllocationAction::Resume, AllocationAction::Leave])),
            None,
        ),
        _ => (None, None),
    }
}

fn confirm_hints(
    proposal_frame: u64,
    last_received_frame: u64,
    current_epoch: u64,
    epoch_length: u64,
) -> (Option<ActionHint>, Option<ActionHint>) {
    if proposal_frame == 0 {
        return (
            Some(now(&[AllocationAction::Reject, AllocationAction::Confirm])),
            None,
        );
    }

    let confirm_epoch = effective_epoch(proposal_frame, epoch_length);
    let start_frame = epoch_start(confirm_epoch, epoch_length);
    let end_frame = epoch_start(confirm_epoch.saturating_add(1), epoch_length);
    let next_action = if current_epoch < confirm_epoch {
        Some(at(
            &[AllocationAction::Reject, AllocationAction::Confirm],
            start_frame,
            last_received_frame,
        ))
    } else if current_epoch == confirm_epoch {
        Some(now(&[AllocationAction::Reject, AllocationAction::Confirm]))
    } else {
        None
    };

    (
        next_action,
        Some(at(
            &[AllocationAction::Expire],
            end_frame,
            last_received_frame,
        )),
    )
}

fn now(actions: &[AllocationAction]) -> ActionHint {
    ActionHint {
        actions: actions.to_vec(),
        timing: ActionTiming::Now,
    }
}

fn at(actions: &[AllocationAction], frame: u64, last_received_frame: u64) -> ActionHint {
    ActionHint {
        actions: actions.to_vec(),
        timing: ActionTiming::At {
            frame,
            frames_remaining: frame.saturating_sub(last_received_frame),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allocation_lifecycle, ActionHint, ActionTiming, AllocationAction, AllocationLifecycle,
        AllocationLifecycleStatus,
    };
    use crate::domain::{AllocationSnapshot, AllocationStatus, EpochSnapshot};

    const LENGTH: u64 = 100;

    fn allocation(status: AllocationStatus) -> AllocationSnapshot {
        AllocationSnapshot {
            filter_hex: "ab".into(),
            status,
            epoch: 2,
            ..Default::default()
        }
    }

    fn epoch(current: u64) -> EpochSnapshot {
        EpochSnapshot {
            current,
            length_frames: LENGTH,
        }
    }

    fn at(action: AllocationAction, frame: u64, last_received_frame: u64) -> ActionHint {
        ActionHint {
            actions: vec![action],
            timing: ActionTiming::At {
                frame,
                frames_remaining: frame.saturating_sub(last_received_frame),
            },
        }
    }

    fn now(actions: &[AllocationAction]) -> ActionHint {
        ActionHint {
            actions: actions.to_vec(),
            timing: ActionTiming::Now,
        }
    }

    #[test]
    fn classifies_every_raw_and_effective_lifecycle_state() {
        struct Case {
            name: &'static str,
            allocation: AllocationSnapshot,
            frame: u64,
            epoch: EpochSnapshot,
            status: AllocationLifecycleStatus,
            label: &'static str,
        }

        let mut activating = allocation(AllocationStatus::Active);
        activating.join_confirm_frame_number = 250;
        activating.epoch = 3;

        let mut departing = allocation(AllocationStatus::Leaving);
        departing.leave_confirm_frame_number = 250;

        let mut expired_join = allocation(AllocationStatus::Joining);
        expired_join.join_frame_number = 150;

        let mut expired_leave = allocation(AllocationStatus::Leaving);
        expired_leave.leave_frame_number = 150;

        let mut expired_epoch = allocation(AllocationStatus::Active);
        expired_epoch.epoch = 1;

        let cases = vec![
            Case {
                name: "unknown",
                allocation: allocation(AllocationStatus::Unknown),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Unknown,
                label: "unknown",
            },
            Case {
                name: "joining",
                allocation: allocation(AllocationStatus::Joining),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Joining,
                label: "joining",
            },
            Case {
                name: "activating",
                allocation: activating,
                frame: 299,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Activating,
                label: "activating",
            },
            Case {
                name: "active",
                allocation: allocation(AllocationStatus::Active),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Active,
                label: "active",
            },
            Case {
                name: "paused",
                allocation: allocation(AllocationStatus::Paused),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Paused,
                label: "paused",
            },
            Case {
                name: "leaving",
                allocation: allocation(AllocationStatus::Leaving),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Leaving,
                label: "leaving",
            },
            Case {
                name: "departing",
                allocation: departing,
                frame: 299,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Departing,
                label: "departing",
            },
            Case {
                name: "rejected",
                allocation: allocation(AllocationStatus::Rejected),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Rejected,
                label: "rejected",
            },
            Case {
                name: "kicked",
                allocation: allocation(AllocationStatus::Kicked),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Kicked,
                label: "kicked",
            },
            Case {
                name: "historic",
                allocation: allocation(AllocationStatus::Historic),
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::Historic,
                label: "historic",
            },
            Case {
                name: "expired joining",
                allocation: expired_join,
                frame: 300,
                epoch: epoch(3),
                status: AllocationLifecycleStatus::ExpiredJoining,
                label: "expiredJoin",
            },
            Case {
                name: "expired leaving",
                allocation: expired_leave,
                frame: 300,
                epoch: epoch(3),
                status: AllocationLifecycleStatus::ExpiredLeaving,
                label: "expiredLeave",
            },
            Case {
                name: "expired epoch",
                allocation: expired_epoch,
                frame: 250,
                epoch: epoch(2),
                status: AllocationLifecycleStatus::ExpiredEpoch,
                label: "re-confirm!",
            },
        ];

        for case in cases {
            let result = allocation_lifecycle(&case.allocation, case.frame, Some(&case.epoch));
            assert_eq!(result.status, case.status, "{} status", case.name);
            assert_eq!(result.label, case.label, "{} label", case.name);
        }
    }

    #[test]
    fn join_confirm_window_changes_at_exact_epoch_boundaries() {
        let mut pending = allocation(AllocationStatus::Joining);
        pending.join_frame_number = 150;

        struct Case {
            name: &'static str,
            frame: u64,
            current_epoch: u64,
            expected: AllocationLifecycle,
        }

        let cases = [
            Case {
                name: "before confirm window",
                frame: 199,
                current_epoch: 1,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Joining,
                    Some(at_many(
                        &[AllocationAction::Reject, AllocationAction::Confirm],
                        200,
                        199,
                    )),
                    Some(at(AllocationAction::Expire, 300, 199)),
                ),
            },
            Case {
                name: "at confirm window start",
                frame: 200,
                current_epoch: 2,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Joining,
                    Some(now(&[AllocationAction::Reject, AllocationAction::Confirm])),
                    Some(at(AllocationAction::Expire, 300, 200)),
                ),
            },
            Case {
                name: "before confirm window end",
                frame: 299,
                current_epoch: 2,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Joining,
                    Some(now(&[AllocationAction::Reject, AllocationAction::Confirm])),
                    Some(at(AllocationAction::Expire, 300, 299)),
                ),
            },
            Case {
                name: "at confirm window end",
                frame: 300,
                current_epoch: 3,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::ExpiredJoining,
                    None,
                    None,
                ),
            },
            Case {
                name: "after confirm window",
                frame: 301,
                current_epoch: 3,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::ExpiredJoining,
                    None,
                    None,
                ),
            },
        ];

        for case in cases {
            assert_eq!(
                allocation_lifecycle(&pending, case.frame, Some(&epoch(case.current_epoch))),
                case.expected,
                "{}",
                case.name
            );
        }
    }

    fn at_many(actions: &[AllocationAction], frame: u64, last_received_frame: u64) -> ActionHint {
        ActionHint {
            actions: actions.to_vec(),
            timing: ActionTiming::At {
                frame,
                frames_remaining: frame.saturating_sub(last_received_frame),
            },
        }
    }

    #[test]
    fn confirmed_transitions_change_at_the_effective_boundary() {
        struct Case {
            name: &'static str,
            allocation: AllocationSnapshot,
            frame: u64,
            current_epoch: u64,
            status: AllocationLifecycleStatus,
            next: Option<ActionHint>,
            default: Option<ActionHint>,
        }

        let mut joined = allocation(AllocationStatus::Active);
        joined.join_confirm_frame_number = 250;
        joined.epoch = 3;
        let mut left = allocation(AllocationStatus::Leaving);
        left.leave_confirm_frame_number = 250;

        let mut cases = Vec::new();
        for (name, frame, current_epoch, status) in [
            (
                "activation before",
                299,
                2,
                AllocationLifecycleStatus::Activating,
            ),
            ("activation at", 300, 3, AllocationLifecycleStatus::Active),
            (
                "activation after",
                301,
                3,
                AllocationLifecycleStatus::Active,
            ),
        ] {
            cases.push(Case {
                name,
                allocation: joined.clone(),
                frame,
                current_epoch,
                status,
                next: Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                default: Some(at(
                    if status == AllocationLifecycleStatus::Activating {
                        AllocationAction::Activate
                    } else {
                        AllocationAction::Renew
                    },
                    if status == AllocationLifecycleStatus::Activating {
                        300
                    } else {
                        400
                    },
                    frame,
                )),
            });
        }
        for (name, frame, current_epoch, status) in [
            (
                "departure before",
                299,
                2,
                AllocationLifecycleStatus::Departing,
            ),
            (
                "departure at",
                300,
                3,
                AllocationLifecycleStatus::ExpiredLeaving,
            ),
            (
                "departure after",
                301,
                3,
                AllocationLifecycleStatus::ExpiredLeaving,
            ),
        ] {
            cases.push(Case {
                name,
                allocation: left.clone(),
                frame,
                current_epoch,
                status,
                next: None,
                default: (status == AllocationLifecycleStatus::Departing)
                    .then(|| at(AllocationAction::Depart, 300, frame)),
            });
        }

        for case in cases {
            let result = allocation_lifecycle(
                &case.allocation,
                case.frame,
                Some(&epoch(case.current_epoch)),
            );
            assert_eq!(result.status, case.status, "{} status", case.name);
            assert_eq!(result.next_action, case.next, "{} next", case.name);
            assert_eq!(result.default_action, case.default, "{} default", case.name);
        }
    }

    #[test]
    fn frame_derived_epoch_controls_status_and_windows_when_rpc_epoch_lags_or_leads() {
        struct Case {
            name: &'static str,
            allocation: AllocationSnapshot,
            frame: u64,
            rpc_epoch: u64,
            expected: AllocationLifecycle,
        }

        let mut joining = allocation(AllocationStatus::Joining);
        joining.join_frame_number = 150;
        let mut active = allocation(AllocationStatus::Active);
        active.epoch = 2;
        let mut confirmed_active = allocation(AllocationStatus::Active);
        confirmed_active.join_confirm_frame_number = 250;
        confirmed_active.epoch = 3;
        let mut confirmed_leave = allocation(AllocationStatus::Leaving);
        confirmed_leave.leave_confirm_frame_number = 250;

        let cases = vec![
            Case {
                name: "lagging RPC epoch does not keep a missed join window open",
                allocation: joining.clone(),
                frame: 300,
                rpc_epoch: 2,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::ExpiredJoining,
                    None,
                    None,
                ),
            },
            Case {
                name: "lagging RPC epoch does not hide a missed reconfirm",
                allocation: active.clone(),
                frame: 300,
                rpc_epoch: 2,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::ExpiredEpoch,
                    Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                    Some(at(AllocationAction::Renew, 400, 300)),
                ),
            },
            Case {
                name: "lagging RPC epoch does not prolong activation",
                allocation: confirmed_active.clone(),
                frame: 300,
                rpc_epoch: 2,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Active,
                    Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                    Some(at(AllocationAction::Renew, 400, 300)),
                ),
            },
            Case {
                name: "lagging RPC epoch does not prolong departure",
                allocation: confirmed_leave.clone(),
                frame: 300,
                rpc_epoch: 2,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::ExpiredLeaving,
                    None,
                    None,
                ),
            },
            Case {
                name: "leading RPC epoch does not expire an open join window",
                allocation: joining,
                frame: 200,
                rpc_epoch: 3,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Joining,
                    Some(now(&[AllocationAction::Reject, AllocationAction::Confirm])),
                    Some(at(AllocationAction::Expire, 300, 200)),
                ),
            },
            Case {
                name: "leading RPC epoch does not falsely expire an active allocation",
                allocation: active,
                frame: 200,
                rpc_epoch: 3,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Active,
                    Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                    Some(at(AllocationAction::Renew, 300, 200)),
                ),
            },
            Case {
                name: "leading RPC epoch does not skip activation",
                allocation: confirmed_active,
                frame: 200,
                rpc_epoch: 3,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Activating,
                    Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                    Some(at(AllocationAction::Activate, 300, 200)),
                ),
            },
            Case {
                name: "leading RPC epoch does not skip departure",
                allocation: confirmed_leave,
                frame: 200,
                rpc_epoch: 3,
                expected: AllocationLifecycle::new(
                    AllocationLifecycleStatus::Departing,
                    None,
                    Some(at(AllocationAction::Depart, 300, 200)),
                ),
            },
        ];

        for case in cases {
            assert_eq!(
                allocation_lifecycle(&case.allocation, case.frame, Some(&epoch(case.rpc_epoch))),
                case.expected,
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn renew_boundary_comes_only_from_the_last_received_frame() {
        let mut active = allocation(AllocationStatus::Active);
        active.epoch = 3;

        for rpc_epoch in [2, 99] {
            let result = allocation_lifecycle(&active, 300, Some(&epoch(rpc_epoch)));

            assert_eq!(
                result.default_action,
                Some(at(AllocationAction::Renew, 400, 300)),
                "rpc epoch {rpc_epoch}"
            );
        }
    }

    #[test]
    fn actions_match_active_paused_leaving_and_stale_epoch_semantics() {
        struct Case {
            name: &'static str,
            allocation: AllocationSnapshot,
            status: AllocationLifecycleStatus,
            next: Option<ActionHint>,
            default: Option<ActionHint>,
        }

        let mut active = allocation(AllocationStatus::Active);
        active.epoch = 2;
        let mut global = active.clone();
        global.filter_hex.clear();
        let paused = allocation(AllocationStatus::Paused);
        let mut leaving = allocation(AllocationStatus::Leaving);
        leaving.leave_frame_number = 150;
        let mut expired = active.clone();
        expired.epoch = 1;

        let cases = [
            Case {
                name: "active",
                allocation: active,
                status: AllocationLifecycleStatus::Active,
                next: Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                default: Some(at(AllocationAction::Renew, 300, 250)),
            },
            Case {
                name: "global is grandfathered",
                allocation: global,
                status: AllocationLifecycleStatus::Active,
                next: Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                default: None,
            },
            Case {
                name: "paused",
                allocation: paused,
                status: AllocationLifecycleStatus::Paused,
                next: Some(now(&[AllocationAction::Resume, AllocationAction::Leave])),
                default: None,
            },
            Case {
                name: "leaving confirm window",
                allocation: leaving,
                status: AllocationLifecycleStatus::Leaving,
                next: Some(now(&[AllocationAction::Reject, AllocationAction::Confirm])),
                default: Some(at(AllocationAction::Expire, 300, 250)),
            },
            Case {
                name: "missed epoch reconfirm",
                allocation: expired,
                status: AllocationLifecycleStatus::ExpiredEpoch,
                next: Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                default: Some(at(AllocationAction::Renew, 300, 250)),
            },
        ];

        for case in cases {
            let result = allocation_lifecycle(&case.allocation, 250, Some(&epoch(2)));
            assert_eq!(result.status, case.status, "{} status", case.name);
            assert_eq!(result.next_action, case.next, "{} next", case.name);
            assert_eq!(result.default_action, case.default, "{} default", case.name);
        }
    }

    #[test]
    fn missing_or_zero_epoch_length_never_assumes_mainnet_720_frame_timing() {
        let mut genesis = allocation(AllocationStatus::Active);
        genesis.epoch = 0;

        let no_epoch = None;
        let zero_length = EpochSnapshot {
            current: 7,
            length_frames: 0,
        };
        let cases = [
            (
                "genesis with valid length",
                Some(&EpochSnapshot {
                    current: 0,
                    length_frames: LENGTH,
                }),
                AllocationLifecycle::new(
                    AllocationLifecycleStatus::Active,
                    Some(now(&[AllocationAction::Pause, AllocationAction::Leave])),
                    Some(at(AllocationAction::Renew, 100, 0)),
                ),
            ),
            (
                "missing epoch",
                no_epoch,
                AllocationLifecycle::new(AllocationLifecycleStatus::Unknown, None, None),
            ),
            (
                "zero epoch length",
                Some(&zero_length),
                AllocationLifecycle::new(AllocationLifecycleStatus::Unknown, None, None),
            ),
        ];

        for (name, epoch, expected) in cases {
            assert_eq!(allocation_lifecycle(&genesis, 0, epoch), expected, "{name}");
        }
    }

    #[test]
    fn missing_epoch_hides_raw_status_and_actions_from_the_lifecycle() {
        struct Case {
            name: &'static str,
            allocation: AllocationSnapshot,
        }

        let mut joining = allocation(AllocationStatus::Joining);
        joining.join_frame_number = 150;
        let mut confirmed_active = allocation(AllocationStatus::Active);
        confirmed_active.join_confirm_frame_number = 250;
        let mut confirmed_leave = allocation(AllocationStatus::Leaving);
        confirmed_leave.leave_confirm_frame_number = 250;
        let cases = [
            Case {
                name: "joining",
                allocation: joining,
            },
            Case {
                name: "confirmed active",
                allocation: confirmed_active,
            },
            Case {
                name: "confirmed leave",
                allocation: confirmed_leave,
            },
        ];

        for case in cases {
            let result = allocation_lifecycle(&case.allocation, 250, None);
            assert_eq!(
                result,
                AllocationLifecycle::new(AllocationLifecycleStatus::Unknown, None, None),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn lifecycle_math_saturates_instead_of_overflowing() {
        let mut joining = allocation(AllocationStatus::Joining);
        joining.join_frame_number = u64::MAX;
        let extreme_epoch = EpochSnapshot {
            current: 1,
            length_frames: u64::MAX,
        };

        let result = allocation_lifecycle(&joining, u64::MAX, Some(&extreme_epoch));
        assert_eq!(result.status, AllocationLifecycleStatus::Joining);
        assert_eq!(
            result.default_action,
            Some(ActionHint {
                actions: vec![AllocationAction::Expire],
                timing: ActionTiming::At {
                    frame: u64::MAX,
                    frames_remaining: 0,
                },
            })
        );
    }
}
