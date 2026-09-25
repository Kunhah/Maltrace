import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { Notification } from "../types";

interface Props {
  caseId: number | null;
  /// Bumped after each run / schedule-firing-refresh so the bell can
  /// re-poll without setInterval polling running constantly.
  refreshKey: number;
}

/// Topbar bell with an unread-count badge. Click to expand a small
/// dropdown of recent notifications; click "Mark all read" to clear
/// the badge.
///
/// Notifications come from scheduled-machine firings + transform
/// errors. The same rows are mirrored to outbound webhooks (Slack /
/// Discord) when configured.
export function NotificationsBell({ caseId, refreshKey }: Props) {
  const [unread, setUnread] = useState(0);
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<Notification[] | null>(null);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (caseId == null) return;
    let cancelled = false;
    api
      .unreadNotificationCount(caseId)
      .then((n) => {
        if (!cancelled) setUnread(n);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [caseId, refreshKey]);

  // Poll while open so a freshly-firing schedule shows up immediately.
  useEffect(() => {
    if (caseId == null || !open) return;
    let cancelled = false;
    const tick = () => {
      api
        .listNotifications(caseId, 30)
        .then((rows) => {
          if (!cancelled) setItems(rows);
        })
        .catch(() => {});
    };
    tick();
    const id = window.setInterval(tick, 15_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [caseId, open, refreshKey]);

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", onClick);
    return () => window.removeEventListener("mousedown", onClick);
  }, [open]);

  const markAllRead = useCallback(async () => {
    if (caseId == null) return;
    try {
      await api.markNotificationsRead(caseId);
      setUnread(0);
    } catch {
      /* swallow */
    }
  }, [caseId]);

  if (caseId == null) return null;

  return (
    <div className="notif-wrap" ref={wrapRef}>
      <button
        className={`topbar-action${unread > 0 ? " has-unread" : ""}`}
        onClick={() => setOpen((o) => !o)}
        title={`${unread} unread notifications`}
        aria-label="Notifications"
      >
        <BellIcon />
        {unread > 0 && (
          <span className="notif-badge">{unread > 99 ? "99+" : unread}</span>
        )}
      </button>
      {open && (
        <div className="notif-dropdown" role="dialog" aria-label="Notifications">
          <header className="notif-head">
            <span>Notifications</span>
            <button
              className="notif-mark-read"
              disabled={unread === 0}
              onClick={markAllRead}
            >
              Mark all read
            </button>
          </header>
          {!items && <p className="muted small">Loading…</p>}
          {items && items.length === 0 && (
            <p className="muted small empty-state">
              Nothing yet. Scheduled machines fire here when they find new
              entities.
            </p>
          )}
          <ul className="notif-list">
            {items?.map((n) => (
              <li
                key={n.id}
                className={`notif-row${n.read_at ? "" : " is-unread"}`}
              >
                <div className="notif-title">{n.title}</div>
                {n.detail && (
                  <div className="notif-detail">{n.detail}</div>
                )}
                <div className="muted small notif-time">
                  {formatTimestamp(n.created_at)}
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function BellIcon() {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
      <path
        d="M8 1.5a3.5 3.5 0 00-3.5 3.5v2.6L3 9.5h10L11.5 7.6V5A3.5 3.5 0 008 1.5zM6.5 11.5a1.5 1.5 0 003 0"
        stroke="currentColor"
        strokeWidth="1.4"
        fill="none"
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </svg>
  );
}

function formatTimestamp(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}
