import { useCallback, useEffect, useRef, useState } from "react";
import type { ConnectionSummary } from "../ipc/types";

export interface ConnectionPickerProps {
  connections: ConnectionSummary[];
  activeId: string | null;
  busy: boolean;
  onSelect(id: string): void;
  onConnect(id: string): void;
  onDisconnect(id: string): void;
  onEdit(c: ConnectionSummary): void;
  onDelete(id: string): void;
  onNew(): void;
}

/**
 * Titlebar connection picker. Always shows the active connection (or "No
 * connection"); click opens a dropdown with the full list plus actions.
 */
export default function ConnectionPicker(props: ConnectionPickerProps) {
  const { connections, activeId, busy } = props;
  const active = connections.find((c) => c.id === activeId) ?? null;
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const handle = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", handle);
    return () => window.removeEventListener("mousedown", handle);
  }, [open]);

  const labelText = active
    ? active.name
    : connections.length === 0
      ? "+ New connection"
      : "Select connection";

  const labelDetail = active
    ? `${active.user}@${active.host}:${active.port}/${active.database}`
    : null;

  const handleRowClick = useCallback(
    (c: ConnectionSummary) => {
      props.onSelect(c.id);
      if (!c.connected) {
        props.onConnect(c.id);
      }
      setOpen(false);
    },
    [props],
  );

  return (
    <div className="conn-picker" ref={rootRef}>
      <button
        className={`conn-picker-button ${active?.connected ? "connected" : ""}`}
        onClick={() => setOpen((o) => !o)}
        disabled={busy && !open}
      >
        <span className={`dot ${active?.connected ? "connected" : ""}`} />
        <span className="conn-picker-label">{labelText}</span>
        {labelDetail && <span className="conn-picker-detail">{labelDetail}</span>}
        <span className="conn-picker-chevron">▾</span>
      </button>
      {open && (
        <div className="conn-picker-menu">
          <div className="conn-picker-menu-header">
            <span>Connections</span>
            <button
              onClick={() => {
                props.onNew();
                setOpen(false);
              }}
            >
              + New
            </button>
          </div>
          <div className="conn-picker-menu-list">
            {connections.length === 0 && (
              <div className="conn-picker-empty">No connection profiles yet.</div>
            )}
            {connections.map((c) => (
              <div
                key={c.id}
                className={`conn-picker-row ${c.id === activeId ? "active" : ""}`}
                onClick={() => handleRowClick(c)}
              >
                <span className={`dot ${c.connected ? "connected" : ""}`} />
                <div className="conn-picker-row-body">
                  <div className="name">{c.name}</div>
                  <div className="meta">
                    {c.user}@{c.host}:{c.port}/{c.database}
                  </div>
                </div>
                <div
                  className="conn-picker-row-actions"
                  onClick={(e) => e.stopPropagation()}
                >
                  {c.connected ? (
                    <button
                      onClick={() => {
                        props.onDisconnect(c.id);
                      }}
                      title="Disconnect"
                    >
                      ◼
                    </button>
                  ) : (
                    <button
                      onClick={() => {
                        props.onConnect(c.id);
                        setOpen(false);
                      }}
                      title="Connect"
                    >
                      ▸
                    </button>
                  )}
                  <button
                    onClick={() => {
                      props.onEdit(c);
                      setOpen(false);
                    }}
                    title="Edit"
                  >
                    ✎
                  </button>
                  <button
                    onClick={() => props.onDelete(c.id)}
                    title="Delete"
                    className="danger-button"
                  >
                    ✕
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
