import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type Phase =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; update: Update; received: number; total: number | null }
  | { kind: "ready"; update: Update }
  | { kind: "error"; message: string }
  | { kind: "uptodate" };

interface Props {
  /**
   * When the user opens the "Check for updates" menu item, the parent bumps
   * this so the banner kicks off a fresh check and surfaces "Up to date"
   * even if no update is available. On automatic startup-check, leave it 0.
   */
  manualCheckToken?: number;
}

/**
 * Auto-update surface. Silently polls for updates on mount; if one is
 * available, surfaces a banner with [Update now] / [Later]. Streams download
 * progress, then prompts to restart. Survives an empty/missing pubkey gracefully:
 * a failed check just shows nothing on startup.
 */
export default function UpdateBanner({ manualCheckToken = 0 }: Props) {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  // Prevent overlapping checks if the user mashes the menu item.
  const inflight = useRef(false);
  const manualRef = useRef(false);

  const runCheck = useCallback(async (manual: boolean) => {
    if (inflight.current) return;
    inflight.current = true;
    manualRef.current = manual;
    setPhase({ kind: "checking" });
    try {
      const update = await check();
      if (update) {
        setPhase({ kind: "available", update });
      } else if (manual) {
        setPhase({ kind: "uptodate" });
      } else {
        setPhase({ kind: "idle" });
      }
    } catch (e: unknown) {
      const message = (e as { message?: string })?.message ?? String(e);
      // Don't pester the user on startup if the endpoint isn't reachable or
      // the pubkey isn't configured yet — only show errors on manual checks.
      if (manual) setPhase({ kind: "error", message });
      else setPhase({ kind: "idle" });
    } finally {
      inflight.current = false;
    }
  }, []);

  // One silent check shortly after mount. Delay slightly so it doesn't race
  // with the initial render / connection workflow.
  useEffect(() => {
    const t = setTimeout(() => void runCheck(false), 2500);
    return () => clearTimeout(t);
  }, [runCheck]);

  // Manual check from the parent menu/button.
  useEffect(() => {
    if (manualCheckToken === 0) return;
    void runCheck(true);
  }, [manualCheckToken, runCheck]);

  const startDownload = useCallback(async () => {
    if (phase.kind !== "available") return;
    const update = phase.update;
    setPhase({ kind: "downloading", update, received: 0, total: null });
    try {
      let downloaded = 0;
      let contentLength: number | null = null;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          contentLength = event.data.contentLength ?? null;
          setPhase({ kind: "downloading", update, received: 0, total: contentLength });
        } else if (event.event === "Progress") {
          downloaded += event.data.chunkLength;
          setPhase({
            kind: "downloading",
            update,
            received: downloaded,
            total: contentLength,
          });
        } else if (event.event === "Finished") {
          setPhase({ kind: "ready", update });
        }
      });
      setPhase({ kind: "ready", update });
    } catch (e: unknown) {
      const message = (e as { message?: string })?.message ?? String(e);
      setPhase({ kind: "error", message });
    }
  }, [phase]);

  const dismiss = useCallback(() => {
    setPhase({ kind: "idle" });
  }, []);

  if (phase.kind === "idle" || phase.kind === "checking") return null;

  return (
    <div className={`update-banner update-banner--${phase.kind}`} role="status">
      {phase.kind === "available" && (
        <>
          <span className="update-icon">↑</span>
          <span className="update-text">
            <strong>Update available</strong> · v{phase.update.version}
            {phase.update.currentVersion ? ` (you have v${phase.update.currentVersion})` : ""}
          </span>
          <span className="update-spacer" />
          <button className="primary" onClick={() => void startDownload()}>
            Update now
          </button>
          <button onClick={dismiss}>Later</button>
        </>
      )}
      {phase.kind === "downloading" && (
        <>
          <span className="update-icon">⤓</span>
          <span className="update-text">
            <strong>Downloading v{phase.update.version}</strong>
            {phase.total
              ? ` · ${formatBytes(phase.received)} / ${formatBytes(phase.total)}`
              : ` · ${formatBytes(phase.received)}`}
          </span>
          <div className="update-progress" aria-hidden>
            <div
              className="update-progress-bar"
              style={{
                width: phase.total
                  ? `${Math.min(100, (phase.received / phase.total) * 100)}%`
                  : "30%",
              }}
            />
          </div>
        </>
      )}
      {phase.kind === "ready" && (
        <>
          <span className="update-icon">✓</span>
          <span className="update-text">
            <strong>Update ready</strong> · restart to apply v{phase.update.version}
          </span>
          <span className="update-spacer" />
          <button className="primary" onClick={() => void relaunch()}>
            Restart now
          </button>
          <button onClick={dismiss}>Later</button>
        </>
      )}
      {phase.kind === "error" && (
        <>
          <span className="update-icon">!</span>
          <span className="update-text">
            <strong>Update check failed</strong> · {phase.message}
          </span>
          <span className="update-spacer" />
          <button onClick={dismiss}>Dismiss</button>
        </>
      )}
      {phase.kind === "uptodate" && (
        <>
          <span className="update-icon">✓</span>
          <span className="update-text">You're on the latest version.</span>
          <span className="update-spacer" />
          <button onClick={dismiss}>Dismiss</button>
        </>
      )}
    </div>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
