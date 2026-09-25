//! Background machine scheduler.
//!
//! Maltrace machines are normally one-shot — you click "Run" and a
//! pipeline of transforms fires once. For threat-intel and continuous-
//! monitoring workflows that's not enough. This module adds a long-
//! running tokio task that wakes up every `TICK_SECONDS`, walks the
//! `machine_schedules` table, and re-runs any machine whose
//! `next_run_at <= now` against a stored seed entity.
//!
//! Each scheduled run is recorded in `machine_schedule_runs` for the UI
//! to surface; new entities since the last run become "alerts" that the
//! notifications panel polls.
//!
//! Schedules use a plain interval (`every_n_seconds`) — no cron
//! expressions. Investigators tend to want simple cadences anyway
//! ("every 6 hours", "daily") and parsing cron in a single line of UI
//! is more pain than it's worth.

use crate::core::SharedState;
use crate::db::MachineSchedule;
use crate::machine;
use chrono::{DateTime, Utc};
use std::time::Duration;

/// Polling interval — how often the scheduler checks the DB for due
/// schedules. 30s is the resolution; schedules with shorter intervals
/// will fire on the next tick after they come due.
pub const TICK_SECONDS: u64 = 30;

pub async fn run_loop(state: SharedState) {
    loop {
        tokio::time::sleep(Duration::from_secs(TICK_SECONDS)).await;
        // Run-due check + executions. Each step swallows its own
        // errors — one bad schedule should never stop the loop.
        if let Err(e) = tick(&state).await {
            eprintln!("maltrace scheduler: tick failed: {e}");
        }
    }
}

async fn tick(state: &SharedState) -> Result<(), String> {
    let now = Utc::now();
    let due: Vec<MachineSchedule> = {
        let db = state.db.lock();
        db.list_due_schedules(now).map_err(crate::core::stringify)?
    };
    for sched in due {
        if let Err(e) = run_one(state, &sched, now).await {
            eprintln!(
                "maltrace scheduler: schedule {} failed: {}",
                sched.id, e
            );
            {
                let db = state.db.lock();
                let _ = db.record_schedule_run(sched.id, now, "error", 0, Some(&e));
                let _ = db.create_notification(
                    sched.case_id,
                    "schedule_error",
                    &format!(
                        "Schedule failed: {} on {}",
                        sched.machine_id, sched.entity_label
                    ),
                    Some(&e),
                    None,
                );
                // Even on error, advance next_run_at so we don't busy-
                // loop on the same broken schedule.
                let next = now + chrono::Duration::seconds(sched.interval_seconds.max(60));
                let _ = db.touch_schedule_after_run(sched.id, now, next);
            }
            // Best-effort outbound webhook. URLs are snapshotted out of
            // the lock; dispatch is fire-and-forget.
            let urls = state.webhooks.urls();
            if !urls.is_empty() {
                let text = format!(
                    "❌ maltrace schedule failed — machine `{}` on *{}*: {}",
                    sched.machine_id, sched.entity_label, e
                );
                crate::webhooks::dispatch_all(&urls, text).await;
            }
        }
    }
    Ok(())
}

async fn run_one(
    state: &SharedState,
    sched: &MachineSchedule,
    now: DateTime<Utc>,
) -> Result<(), String> {
    // Scheduler-initiated runs are attributed to a special "scheduler"
    // actor so the audit log can distinguish autonomous re-runs from
    // user actions.
    let result =
        machine::run_as(state, &sched.machine_id, &sched.entity_id, Some("scheduler")).await?;
    let new_count = result.new_entities.len() as i64;
    let new_labels: Vec<String> = result
        .new_entities
        .iter()
        .take(5)
        .map(|e| format!("{} {}", e.entity_type, e.label))
        .collect();
    {
        let db = state.db.lock();
        let _ = db.record_schedule_run(sched.id, now, "ok", new_count, None);
        if new_count > 0 {
            // Surface every new-entity firing in the bell. A "0
            // entities" firing is uninteresting and stays out of the
            // notification stream.
            let detail = if new_labels.is_empty() {
                None
            } else {
                Some(new_labels.join(", "))
            };
            let _ = db.create_notification(
                sched.case_id,
                "schedule_new_entities",
                &format!(
                    "{} new {} on `{}` from `{}`",
                    new_count,
                    if new_count == 1 { "entity" } else { "entities" },
                    sched.entity_label,
                    sched.machine_id
                ),
                detail.as_deref(),
                None,
            );
        }
        let next = now + chrono::Duration::seconds(sched.interval_seconds.max(60));
        db.touch_schedule_after_run(sched.id, now, next)
            .map_err(crate::core::stringify)?;
    }
    // Webhook notification on new entities only — silent firings are
    // not interesting enough to ping the chat channel.
    if new_count > 0 {
        let urls = state.webhooks.urls();
        if !urls.is_empty() {
            let preview = if new_labels.is_empty() {
                String::new()
            } else {
                format!("\n• {}", new_labels.join("\n• "))
            };
            let text = format!(
                "🔔 maltrace — `{}` on *{}* found {} new {}{}",
                sched.machine_id,
                sched.entity_label,
                new_count,
                if new_count == 1 { "entity" } else { "entities" },
                preview,
            );
            crate::webhooks::dispatch_all(&urls, text).await;
        }
    }
    Ok(())
}

