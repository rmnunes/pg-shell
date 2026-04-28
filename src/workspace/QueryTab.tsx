import { useCallback, useMemo, useRef } from "react";
import QueryEditor, { type QueryEditorHandle } from "../editor/QueryEditor";
import CommandsPanel from "../results/CommandsPanel";
import ResultsGrid from "../results/ResultsGrid";
import {
  downloadText,
  rowsToCsv,
  rowsToJson,
  rowsToTsv,
} from "../results/exporter";
import {
  queryCancel,
  queryExecute,
  subscribeQuery,
} from "../ipc/query";
import { useTabStore, type QueryTabState } from "./tabs";

interface Props {
  tab: QueryTabState;
}

/**
 * Editor + results + toolbar for a single tab. Stateless: it reads/writes
 * the tab's state via the Zustand store so tabs keep their results when the
 * user switches away.
 */
export default function QueryTab({ tab }: Props) {
  const updateSql = useTabStore((s) => s.updateSql);
  const setRunning = useTabStore((s) => s.setRunning);
  const setColumns = useTabStore((s) => s.setColumns);
  const appendRows = useTabStore((s) => s.appendRows);
  const appendCommand = useTabStore((s) => s.appendCommand);
  const setDone = useTabStore((s) => s.setDone);
  const setError = useTabStore((s) => s.setError);

  const editorRef = useRef<QueryEditorHandle | null>(null);

  const run = useCallback(
    async (overrideSql?: string) => {
      if (tab.runState.phase === "running") return;
      const text = overrideSql ?? tab.sql;
      if (!text.trim()) return;
      const mode: "selection" | "buffer" =
        overrideSql !== undefined && overrideSql !== tab.sql ? "selection" : "buffer";

      const qid = crypto.randomUUID();
      // Pre-attach subscribers before dispatching execute so we don't miss
      // the initial start event on fast queries.
      const unlisten = await subscribeQuery(qid, {
        onStart: (e) => setColumns(tab.id, e.columns, e.backend_pid),
        onRows: (e) => appendRows(tab.id, e.rows),
        onCommand: (e) =>
          appendCommand(tab.id, { index: e.index, rows_affected: e.rows_affected }),
        onDone: (e) => setDone(tab.id, e.total_rows, e.duration_ms, e.cancelled),
        onError: (e) => setError(tab.id, e.message),
      });
      setRunning(tab.id, qid, text, mode, unlisten);

      try {
        await queryExecute(tab.profileId, text, qid);
      } catch (e: unknown) {
        const err = e as { message?: string };
        setError(tab.id, err.message ?? String(e));
      }
    },
    [tab.id, tab.profileId, tab.sql, tab.runState.phase, setRunning, setColumns, appendRows, setDone, setError],
  );

  const cancel = useCallback(async () => {
    if (tab.activeQueryId) await queryCancel(tab.activeQueryId);
  }, [tab.activeQueryId]);

  const runFromToolbar = useCallback(() => {
    const text = editorRef.current?.getRunText() ?? tab.sql;
    void run(text === tab.sql ? undefined : text);
  }, [run, tab.sql]);

  const statusLine = useMemo(() => statusText(tab), [tab]);

  const exportAs = (kind: "csv" | "tsv" | "json") => {
    if (!tab.columns.length || !tab.rows.length) return;
    const ts = new Date().toISOString().replace(/[:.]/g, "-");
    if (kind === "csv") {
      downloadText(`pg-shell-${ts}.csv`, "text/csv", rowsToCsv(tab.columns, tab.rows));
    } else if (kind === "tsv") {
      navigator.clipboard.writeText(rowsToTsv(tab.columns, tab.rows)).catch(() => undefined);
    } else {
      downloadText(`pg-shell-${ts}.json`, "application/json", rowsToJson(tab.columns, tab.rows));
    }
  };

  const running = tab.runState.phase === "running";

  return (
    <div className="query-tab">
      <div className="query-toolbar">
        {running ? (
          <button className="danger" onClick={cancel}>
            ■ Cancel
          </button>
        ) : (
          <button
            className="primary"
            onClick={runFromToolbar}
            title="F5 or Ctrl+Enter · runs selection if any"
          >
            ▶ Run (F5)
          </button>
        )}
        <span className="toolbar-sep" />
        <button disabled={!tab.rows.length || running} onClick={() => exportAs("csv")}>
          Export CSV
        </button>
        <button
          disabled={!tab.rows.length || running}
          onClick={() => exportAs("tsv")}
          title="Copy as TSV to clipboard"
        >
          Copy TSV
        </button>
        <button disabled={!tab.rows.length || running} onClick={() => exportAs("json")}>
          Export JSON
        </button>
        <span className="toolbar-spacer" />
        <span className="toolbar-status">{statusLine}</span>
      </div>
      <div className="query-split">
        <div className="query-editor-pane">
          <QueryEditor
            ref={editorRef}
            value={tab.sql}
            onChange={(v) => updateSql(tab.id, v)}
            onRun={(text) => void run(text === tab.sql ? undefined : text)}
            readOnly={running}
            profileId={tab.profileId}
          />
        </div>
        <div className="query-results-pane">
          {tab.runState.phase === "error" ? (
            <div className="query-error">{tab.runState.message}</div>
          ) : tab.columns.length ? (
            <ResultsGrid columns={tab.columns} rows={tab.rows} />
          ) : tab.commands.length ? (
            <CommandsPanel
              sql={tab.executedSql}
              commands={tab.commands}
              durationMs={
                tab.runState.phase === "done" ? tab.runState.durationMs : 0
              }
              cancelled={tab.runState.phase === "done" && tab.runState.cancelled}
            />
          ) : (
            <div className="query-placeholder">
              {running ? "Executing…" : "Run a query to see results."}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function statusText(tab: QueryTabState): string {
  const liveRows = tab.rows.length;
  switch (tab.runState.phase) {
    case "idle":
      return "Ready";
    case "running": {
      const scope = tab.runState.mode === "selection" ? "selection · " : "";
      return `${scope}Running${tab.runState.pid ? ` · pid ${tab.runState.pid}` : ""} · ${liveRows.toLocaleString()} rows`;
    }
    case "done": {
      const scope = tab.runState.mode === "selection" ? "selection · " : "";
      return tab.runState.cancelled
        ? `${scope}Cancelled after ${tab.runState.durationMs}ms · ${tab.runState.rowCount.toLocaleString()} rows`
        : `${scope}Done · ${tab.runState.durationMs}ms · ${tab.runState.rowCount.toLocaleString()} rows`;
    }
    case "error":
      return `Error: ${tab.runState.message}`;
  }
}
