import { useEffect, useState } from "react";
import {
  connectionCreate,
  connectionEntraSignOut,
  connectionPasswordSet,
  connectionTest,
  connectionTestTransient,
  connectionUpdate,
  onEntraSignIn,
} from "../ipc";
import type {
  AuthMethod,
  ConnectionSummary,
  ProfileInput,
  SslMode,
  TestOutcome,
} from "../ipc/types";

interface Props {
  initial: ConnectionSummary | null;
  onClose: () => void;
  onSaved: () => void;
}

type TestStatus =
  | { kind: "idle" }
  | { kind: "running"; message: string }
  | { kind: "ok"; result: TestOutcome }
  | { kind: "err"; message: string };

const BROWSER_WAIT = "Complete the Microsoft sign-in in your browser…";

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
    auth_method: initial?.auth_method ?? "password",
    entra: initial?.entra ?? null,
  });
  const [password, setPassword] = useState("");
  const [status, setStatus] = useState<TestStatus>({ kind: "idle" });
  const [saving, setSaving] = useState(false);
  const [signedIn, setSignedIn] = useState(
    Boolean(initial && initial.auth_method === "entra_mfa" && initial.has_password),
  );

  const isEntra = form.auth_method === "entra_mfa";
  const hasStoredPassword = Boolean(
    initial && initial.auth_method === "password" && initial.has_password,
  );

  // When a running test hands off to the browser, say so instead of "Testing…".
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    onEntraSignIn(() => {
      setStatus((s) => (s.kind === "running" ? { kind: "running", message: BROWSER_WAIT } : s));
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const canSubmit =
    form.name.trim().length > 0 &&
    form.host.trim().length > 0 &&
    form.database.trim().length > 0 &&
    // Entra profiles may leave User blank: the role is the signed-in account.
    (isEntra || form.user.trim().length > 0);

  const setAuthMethod = (method: AuthMethod) => {
    if (method === "entra_mfa") {
      setForm({
        ...form,
        auth_method: method,
        // The password-auth default role makes no sense for Entra.
        user: form.user === "postgres" ? "" : form.user,
        entra: form.entra ?? { tenant: null, client_id: null },
        // Azure refuses plaintext connections; don't let a default trip people up.
        ssl_mode:
          form.ssl_mode === "disable" || form.ssl_mode === "prefer" ? "require" : form.ssl_mode,
      });
    } else {
      setForm({
        ...form,
        auth_method: method,
        user: form.user.trim().length ? form.user : "postgres",
        entra: null,
      });
    }
  };

  const setEntra = (patch: Partial<NonNullable<ProfileInput["entra"]>>) => {
    setForm({
      ...form,
      entra: { tenant: null, client_id: null, ...(form.entra ?? {}), ...patch },
    });
  };

  const handleTest = async () => {
    if (!isEntra && !password && !hasStoredPassword) {
      setStatus({ kind: "err", message: "Enter a password to test." });
      return;
    }
    setStatus({ kind: "running", message: isEntra ? "Signing in…" : "Testing…" });
    try {
      const result = initial
        ? await connectionTest(initial.id, password || undefined)
        : await connectionTestTransient(form, isEntra ? null : password);
      setStatus({ kind: "ok", result });
      if (initial && isEntra) setSignedIn(true);
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
        if (!isEntra && password.length > 0) {
          await connectionPasswordSet(initial.id, password);
        }
      } else {
        const created = await connectionCreate(form, isEntra ? undefined : password || undefined);
        if (!isEntra && password.length > 0 && !created.has_password) {
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

  const handleSignOut = async () => {
    if (!initial) return;
    try {
      await connectionEntraSignOut(initial.id);
      setSignedIn(false);
      setStatus({ kind: "idle" });
    } catch (e: unknown) {
      const err = e as { message?: string };
      setStatus({ kind: "err", message: err.message ?? String(e) });
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
          <div className="full">
            <label>Authentication</label>
            <select
              value={form.auth_method}
              onChange={(e) => setAuthMethod(e.target.value as AuthMethod)}
            >
              <option value="password">Password</option>
              <option value="entra_mfa">Microsoft Entra MFA</option>
            </select>
          </div>
          <div>
            <label>Host</label>
            <input
              value={form.host}
              onChange={(e) => setForm({ ...form, host: e.target.value })}
              placeholder={isEntra ? "myserver.postgres.database.azure.com" : undefined}
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
            <label>{isEntra ? "User (optional)" : "User"}</label>
            <input
              value={form.user}
              onChange={(e) => setForm({ ...form, user: e.target.value })}
              placeholder={isEntra ? "signed-in account" : undefined}
            />
          </div>
          {isEntra ? (
            <>
              <div>
                <label>Tenant (optional)</label>
                <input
                  value={form.entra?.tenant ?? ""}
                  placeholder="organizations"
                  onChange={(e) =>
                    setEntra({ tenant: e.target.value.length ? e.target.value : null })
                  }
                />
              </div>
              <div>
                <label>Client ID (optional)</label>
                <input
                  value={form.entra?.client_id ?? ""}
                  placeholder="Azure CLI public client"
                  onChange={(e) =>
                    setEntra({ client_id: e.target.value.length ? e.target.value : null })
                  }
                />
              </div>
              <div className="full">
                <label>Sign-in</label>
                <div className="hint">
                  {signedIn ? (
                    <>
                      Signed in — the cached session is reused when connecting.{" "}
                      <button type="button" className="link-button" onClick={handleSignOut}>
                        Sign out
                      </button>
                    </>
                  ) : (
                    "You'll sign in through your browser when connecting; MFA and Conditional Access are handled there. Leave User blank to connect as the account you sign in with, or enter an Entra group's name to connect as that group's role. Only a refresh token is cached, in the OS keychain."
                  )}
                </div>
              </div>
            </>
          ) : (
            <div className="full">
              <label>
                Password {hasStoredPassword && <i>(stored — leave blank to keep)</i>}
              </label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={hasStoredPassword ? "••••••••" : ""}
              />
            </div>
          )}
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
            {status.kind === "running" && status.message}
            {status.kind === "ok" &&
              `OK · ${status.result.latency_ms}ms · ${status.result.server.server_version.split(" ").slice(0, 2).join(" ")}`}
            {status.kind === "err" && status.message}
          </span>
          <button disabled={!canSubmit || status.kind === "running"} onClick={handleTest}>
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
