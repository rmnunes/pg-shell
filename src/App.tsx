import { useCallback, useEffect, useState } from "react";
import ConnectionDialog from "./components/ConnectionDialog";
import ConnectionPicker from "./components/ConnectionPicker";
import UpdateBanner from "./components/UpdateBanner";
import Workspace from "./workspace/Workspace";
import ObjectPanel from "./tree/ObjectPanel";
import {
  objectDefinition,
  scriptAsInsert,
  scriptAsSelect,
  type ObjectKind,
} from "./ipc/schema";
import {
  connectionConnect,
  connectionDelete,
  connectionDisconnect,
  connectionsList,
} from "./ipc";
import type { ConnectionSummary, ServerInfo } from "./ipc/types";

type DialogState =
  | { mode: "closed" }
  | { mode: "create" }
  | { mode: "edit"; profile: ConnectionSummary };

interface InjectedSql {
  text: string;
  version: number;
}

export default function App() {
  const [connections, setConnections] = useState<ConnectionSummary[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [server, setServer] = useState<ServerInfo | null>(null);
  const [dialog, setDialog] = useState<DialogState>({ mode: "closed" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [injectedSql, setInjectedSql] = useState<InjectedSql | null>(null);
  const [updateCheckToken, setUpdateCheckToken] = useState(0);

  const refresh = useCallback(async () => {
    const list = await connectionsList();
    setConnections(list);
  }, []);

  useEffect(() => {
    refresh().catch((e) => setError(String(e?.message ?? e)));
  }, [refresh]);

  const active = connections.find((c) => c.id === activeId) ?? null;

  const handleConnect = async (id: string) => {
    setBusy(true);
    setError(null);
    try {
      const info = await connectionConnect(id);
      setServer(info);
      setActiveId(id);
      await refresh();
    } catch (e: unknown) {
      const err = e as { message?: string };
      setError(err.message ?? String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleDisconnect = async (id: string) => {
    setBusy(true);
    try {
      await connectionDisconnect(id);
      if (activeId === id) {
        setServer(null);
      }
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!confirm("Delete this connection profile? The saved password will also be removed.")) {
      return;
    }
    setBusy(true);
    try {
      await connectionDelete(id);
      if (activeId === id) {
        setActiveId(null);
        setServer(null);
      }
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const injectText = (text: string) => {
    setInjectedSql({ text, version: Date.now() });
  };

  const handleScriptFromPanel = async (
    target: { schema: string; name: string; kind: string },
    action: "select" | "insert",
  ) => {
    if (!active) return;
    try {
      const text =
        action === "select"
          ? await scriptAsSelect(active.id, target.schema, target.name)
          : await scriptAsInsert(active.id, target.schema, target.name);
      injectText(text);
    } catch (e: unknown) {
      const err = e as { message?: string };
      setError(err.message ?? String(e));
    }
  };

  const handleViewDefinition = async (target: { schema: string; name: string; kind: string }) => {
    if (!active) return;
    const kind = objectKindOf(target.kind);
    if (!kind) return;
    try {
      const text = await objectDefinition(active.id, kind, target.schema, target.name);
      injectText(text);
    } catch (e: unknown) {
      const err = e as { message?: string };
      setError(err.message ?? String(e));
    }
  };

  return (
    <div className="app">
      <div className="titlebar">
        <span className="titlebar-brand">pg-shell</span>
        <ConnectionPicker
          connections={connections}
          activeId={activeId}
          busy={busy}
          onSelect={setActiveId}
          onConnect={(id) => void handleConnect(id)}
          onDisconnect={(id) => void handleDisconnect(id)}
          onEdit={(c) => setDialog({ mode: "edit", profile: c })}
          onDelete={(id) => void handleDelete(id)}
          onNew={() => setDialog({ mode: "create" })}
        />
        <span className="titlebar-spacer" />
        <button
          className="titlebar-button"
          onClick={() => setUpdateCheckToken((t) => t + 1)}
          title="Check for updates"
        >
          ↑ Check for updates
        </button>
      </div>
      <UpdateBanner manualCheckToken={updateCheckToken} />
      <div className="main">
        <aside className="sidebar">
          {active?.connected ? (
            <ObjectPanel
              profileId={active.id}
              onOpenScript={(t, a) => void handleScriptFromPanel(t, a)}
              onViewDefinition={(t) => void handleViewDefinition(t)}
            />
          ) : (
            <div className="sidebar-placeholder">
              {connections.length === 0
                ? "Add a connection to get started."
                : "Pick a connection above to browse objects."}
            </div>
          )}
        </aside>
        <section className="workspace">
          {active?.connected ? (
            <Workspace
              profileId={active.id}
              connections={connections}
              injectedSql={injectedSql}
            />
          ) : server ? (
            <div className="empty-hint">
              <h2>Connected</h2>
              <div style={{ fontSize: 12 }}>{server.server_version}</div>
            </div>
          ) : (
            <div className="empty-hint">
              <h2>Welcome to pg-shell</h2>
              <p>
                Click the connection picker above to add a server and connect. Passwords live
                in the OS keychain — never in plaintext.
              </p>
            </div>
          )}
        </section>
      </div>
      <div className="statusbar">
        <span>
          <span className={`dot ${server ? "connected" : ""}`} />
          {server ? `${active?.name ?? ""} · ${server.server_version.split(" ").slice(0, 2).join(" ")}` : "Not connected"}
        </span>
        <span>{error ?? "Ready"}</span>
      </div>
      {dialog.mode !== "closed" && (
        <ConnectionDialog
          initial={dialog.mode === "edit" ? dialog.profile : null}
          onClose={() => setDialog({ mode: "closed" })}
          onSaved={async () => {
            setDialog({ mode: "closed" });
            await refresh();
          }}
        />
      )}
    </div>
  );
}

function objectKindOf(kind: string): ObjectKind | null {
  switch (kind) {
    case "table":
    case "partitioned_table":
    case "foreign_table":
      return "table";
    case "view":
      return "view";
    case "materialized_view":
      return "materialized_view";
    case "function":
    case "aggregate":
    case "window":
      return "function";
    case "procedure":
      return "procedure";
    case "sequence":
      return "sequence";
    case "trigger":
      return "trigger";
    case "index":
    case "primary_key_index":
    case "unique_index":
      return "index";
    default:
      return null;
  }
}
