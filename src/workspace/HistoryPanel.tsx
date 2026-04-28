import { useCallback, useEffect, useState } from "react";
import { historyClear, historyList, type HistoryEntry } from "../ipc/history";

export interface HistoryPanelProps {
  profileId: string;
  onOpen(sql: string): void;
  onClose(): void;
}

/**
 * Full-screen overlay listing recent queries for the active profile. The
 * list is fetched fresh on open and on every keystroke in the search box —
 * SQLite handles the small profile-scoped dataset in well under a frame.
 *
 * Selecting an entry closes the panel and opens the SQL in a new tab. Enter
 * on the focused row does the same; Esc closes without selection.
 */
export default function HistoryPanel({ profileId, onOpen, onClose }: HistoryPanelProps) {
  const [entries, setEntries] = useState<HistoryEntry[] | null>(null);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await historyList(profileId, search);
      setEntries(list);
      setSelected(0);
      setError(null);
    } catch (e: unknown) {
      const err = e as { message?: string };
      setError(err.message ?? String(e));
    }
  }, [profileId, search]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onKey = useCallback(
    (e: React.KeyboardEvent) => {
      if (!entries?.length) {
        if (e.key === "Escape") onClose();
        return;
      }
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelected((s) => Math.min(entries.length - 1, s + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelected((s) => Math.max(0, s - 1));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const entry = entries[selected];
        if (entry) {
          onOpen(entry.sql);
        }
      }
    },
    [entries, selected, onOpen, onClose],
  );

  const handleClear = async () => {
    if (!confirm("Clear all query history for this connection?")) return;
    try {
      await historyClear(profileId);
      await refresh();
    } catch (e: unknown) {
      const err = e as { message?: string };
      setError(err.message ?? String(e));
    }
  };

  return (
    <div className="history-overlay" onClick={onClose}>
      <div className="history-panel" onClick={(e) => e.stopPropagation()} onKeyDown={onKey}>
        <div className="history-header">
          <input
            autoFocus
            className="history-search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search history…"
          />
          <button onClick={handleClear} className="danger" title="Clear history for this connection">
            Clear
          </button>
          <button onClick={onClose} title="Close (Esc)">
            ✕
          </button>
        </div>
        {error && <div className="history-error">{error}</div>}
        <div className="history-list">
          {entries == null && !error && <div className="history-hint">Loading…</div>}
          {entries && entries.length === 0 && !error && (
            <div className="history-hint">
              {search ? "No matches." : "No queries executed on this connection yet."}
            </div>
          )}
          {entries?.map((h, i) => (
            <HistoryRow
              key={h.id}
              entry={h}
              active={i === selected}
              onActivate={() => setSelected(i)}
              onPick={() => onOpen(h.sql)}
            />
          ))}
        </div>
        <div className="history-footer">
          <span>↑↓ move · Enter open · Esc close</span>
          {entries && <span>{entries.length} entries</span>}
        </div>
      </div>
    </div>
  );
}

function HistoryRow({
  entry,
  active,
  onActivate,
  onPick,
}: {
  entry: HistoryEntry;
  active: boolean;
  onActivate(): void;
  onPick(): void;
}) {
  const preview = entry.sql.replace(/\s+/g, " ").slice(0, 160).trim();
  const ts = new Date(entry.started_at * 1000);
  const status = statusOf(entry);
  return (
    <div
      className={`history-row ${active ? "active" : ""} status-${status.kind}`}
      onMouseEnter={onActivate}
      onClick={onPick}
      title={entry.sql}
    >
      <div className="history-preview">{preview}</div>
      <div className="history-meta">
        <span className={`history-status ${status.kind}`}>{status.label}</span>
        <span className="history-time">{formatTs(ts)}</span>
      </div>
    </div>
  );
}

function statusOf(e: HistoryEntry): { kind: string; label: string } {
  if (e.error) return { kind: "error", label: `Error · ${short(e.error, 64)}` };
  if (e.cancelled) return { kind: "cancelled", label: "Cancelled" };
  if (e.duration_ms == null) return { kind: "pending", label: "In flight" };
  const rows = e.row_count ?? 0;
  const dur = `${e.duration_ms} ms`;
  return { kind: "ok", label: `${rows.toLocaleString()} rows · ${dur}` };
}

function short(s: string, n: number): string {
  return s.length > n ? s.slice(0, n - 1) + "…" : s;
}

function formatTs(d: Date): string {
  const today = new Date();
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  const t = d.toLocaleTimeString(undefined, { hour12: false });
  if (sameDay) return t;
  const dateStr = d.toLocaleDateString();
  return `${dateStr} ${t}`;
}
