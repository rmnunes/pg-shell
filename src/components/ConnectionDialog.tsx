import { useState } from "react";
import {
  connectionCreate,
  connectionPasswordSet,
  connectionTest,
  connectionUpdate,
} from "../ipc";
import type { ConnectionSummary, ProfileInput, SslMode, TestOutcome } from "../ipc/types";

interface Props {
  initial: ConnectionSummary | null;
  onClose: () => void;
  onSaved: () => void;
}

type TestStatus =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; result: TestOutcome }
  | { kind: "err"; message: string };

export default function ConnectionDialog({ initial, onClose, onSaved }: Props) {
  const [form, setForm] = useState<ProfileInput>({
    name: initial?.name ?? "",
    host: initial?.host ?? "localhost",
    port: initial?.port ?? 5432,
    database: initial?.database ?? "postgres",
    user: initial?.user ?? "postgres",
    ssl_mode: initial?.ssl_mode ?? "prefer",
    app_name: initial?.app_name ?? null,
    group: initial?.group ?? null,
  });
  const [password, setPassword] = useState("");
  const [status, setStatus] = useState<TestStatus>({ kind: "idle" });
  const [saving, setSaving] = useState(false);

  const canSubmit =
    form.name.trim().length > 0 &&
    form.host.trim().length > 0 &&
    form.database.trim().length > 0 &&
    form.user.trim().length > 0;

  const handleTest = async () => {
    if (!initial && !password) {
      setStatus({ kind: "err", message: "Enter a password to test a new profile." });
      return;
    }
    setStatus({ kind: "running" });
    try {
      if (initial) {
        // For existing profile, keychain lookup happens server-side when password is empty
        const result = await connectionTest(initial.id, password || undefined);
        setStatus({ kind: "ok", result });
      } else {
        // Creating: test by round-tripping via a temporary create path — simpler, just
        // create then test. Instead, we require the user to save first for now; gate
        // Test to existing profiles until a transient test IPC lands.
        setStatus({
          kind: "err",
          message: "Save the profile first, then Test.",
        });
      }
    } catch (e: unknown) {
      const err = e as { message?: string };
      setStatus({ kind: "err", message: err.message ?? String(e) });
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      if (initial) {
        await connectionUpdate(initial.id, form);
        if (password.length > 0) {
          await connectionPasswordSet(initial.id, password);
        }
      } else {
        const created = await connectionCreate(form, password || undefined);
        if (password.length > 0 && !created.has_password) {
          await connectionPasswordSet(created.id, password);
        }
      }
      onSaved();
    } catch (e: unknown) {
      const err = e as { message?: string };
      setStatus({ kind: "err", message: err.message ?? String(e) });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="dialog-overlay" onMouseDown={onClose}>
      <div className="dialog" onMouseDown={(e) => e.stopPropagation()}>
        <div className="dialog-header">
          {initial ? `Edit ${initial.name}` : "New connection"}
        </div>
        <div className="dialog-body">
          <div className="full">
            <label>Name</label>
            <input
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              autoFocus
            />
          </div>
          <div>
            <label>Host</label>
            <input
              value={form.host}
              onChange={(e) => setForm({ ...form, host: e.target.value })}
            />
          </div>
          <div>
            <label>Port</label>
            <input
              type="number"
              value={form.port}
              onChange={(e) =>
                setForm({ ...form, port: Number.parseInt(e.target.value, 10) || 5432 })
              }
            />
          </div>
          <div>
            <label>Database</label>
            <input
              value={form.database}
              onChange={(e) => setForm({ ...form, database: e.target.value })}
            />
          </div>
          <div>
            <label>User</label>
            <input
              value={form.user}
              onChange={(e) => setForm({ ...form, user: e.target.value })}
            />
          </div>
          <div className="full">
            <label>Password {initial?.has_password && <i>(stored — leave blank to keep)</i>}</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={initial?.has_password ? "••••••••" : ""}
            />
          </div>
          <div>
            <label>SSL mode</label>
            <select
              value={form.ssl_mode}
              onChange={(e) => setForm({ ...form, ssl_mode: e.target.value as SslMode })}
            >
              <option value="disable">disable</option>
              <option value="prefer">prefer</option>
              <option value="require">require</option>
              <option value="verify_ca">verify-ca</option>
              <option value="verify_full">verify-full</option>
            </select>
          </div>
          <div>
            <label>App name</label>
            <input
              value={form.app_name ?? ""}
              placeholder="pg-shell"
              onChange={(e) =>
                setForm({ ...form, app_name: e.target.value.length ? e.target.value : null })
              }
            />
          </div>
          <div className="full">
            <label>Group (optional)</label>
            <input
              value={form.group ?? ""}
              placeholder="e.g. prod / staging"
              onChange={(e) =>
                setForm({ ...form, group: e.target.value.length ? e.target.value : null })
              }
            />
          </div>
        </div>
        <div className="dialog-footer">
          <span
            className={`status ${status.kind === "ok" ? "ok" : status.kind === "err" ? "err" : ""}`}
          >
            {status.kind === "running" && "Testing…"}
            {status.kind === "ok" &&
              `OK · ${status.result.latency_ms}ms · ${status.result.server.server_version.split(" ").slice(0, 2).join(" ")}`}
            {status.kind === "err" && status.message}
          </span>
          <button disabled={!initial || status.kind === "running"} onClick={handleTest}>
            Test
          </button>
          <button onClick={onClose}>Cancel</button>
          <button className="primary" disabled={!canSubmit || saving} onClick={handleSave}>
            {saving ? "Saving…" : initial ? "Save" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}
