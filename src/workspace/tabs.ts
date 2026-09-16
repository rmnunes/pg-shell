import { create } from "zustand";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { ColumnMeta, Row } from "../ipc/query";
import type { CommandResult } from "../results/CommandsPanel";

export type TabRunState =
  | { phase: "idle" }
  | { phase: "running"; startedAt: number; pid: number | null; mode: "selection" | "buffer" }
  | { phase: "done"; durationMs: number; rowCount: number; cancelled: boolean; mode: "selection" | "buffer" }
  | { phase: "error"; message: string };

export interface ResultSet {
  columns: ColumnMeta[];
  rows: Row[];
}

export interface QueryTabState {
  id: string;
  title: string;
  profileId: string;
  sql: string;
  /** SQL actually executed — preserved so the commands panel can derive tags
   *  even after the user keeps editing the buffer. */
  executedSql: string;
  /** One entry per result-set-producing statement in the last batch. A script
   *  ending in several SELECTs yields several, each with its own shape;
   *  flattening them into one grid mismatches rows against columns. */
  results: ResultSet[];
  /** Which result set the grid is showing. */
  activeResult: number;
  commands: CommandResult[];
  runState: TabRunState;
  activeQueryId: string | null;
  /** Attached listener for the in-flight query, if any. */
  unlisten: UnlistenFn | null;
  /** Whether the buffer has unsaved changes vs. last committed SQL. Cheap
   *  dirty indicator for the tab strip "●". */
  dirty: boolean;
}

interface TabStore {
  tabs: QueryTabState[];
  activeId: string | null;

  openTab(profileId: string, initialSql?: string): string;
  closeTab(id: string): void;
  setActive(id: string): void;
  updateSql(id: string, sql: string): void;
  renameTab(id: string, title: string): void;

  // Query lifecycle
  setRunning(
    id: string,
    queryId: string,
    executedSql: string,
    mode: "selection" | "buffer",
    unlisten: UnlistenFn | null,
  ): void;
  startResult(id: string, resultIndex: number, cols: ColumnMeta[], pid: number): void;
  appendRows(id: string, resultIndex: number, rows: Row[]): void;
  setActiveResult(id: string, resultIndex: number): void;
  appendCommand(id: string, cmd: CommandResult): void;
  setDone(id: string, rowCount: number, durationMs: number, cancelled: boolean): void;
  setError(id: string, message: string): void;
  resetResults(id: string): void;
}

let tabSeq = 1;

export const useTabStore = create<TabStore>((set, get) => ({
  tabs: [],
  activeId: null,

  openTab(profileId, initialSql) {
    const id = crypto.randomUUID();
    const tab: QueryTabState = {
      id,
      title: `Query ${tabSeq++}`,
      profileId,
      sql: initialSql ?? "",
      executedSql: "",
      results: [],
      activeResult: 0,
      commands: [],
      runState: { phase: "idle" },
      activeQueryId: null,
      unlisten: null,
      dirty: false,
    };
    set((s) => ({
      tabs: [...s.tabs, tab],
      activeId: id,
    }));
    return id;
  },

  closeTab(id) {
    const tab = get().tabs.find((t) => t.id === id);
    if (tab?.unlisten) {
      try {
        tab.unlisten();
      } catch {
        // ignore
      }
    }
    set((s) => {
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeId =
        s.activeId === id ? (tabs.length ? tabs[tabs.length - 1].id : null) : s.activeId;
      return { tabs, activeId };
    });
  },

  setActive(id) {
    set({ activeId: id });
  },

  updateSql(id, sql) {
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, sql, dirty: true } : t)),
    }));
  },

  renameTab(id, title) {
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, title } : t)),
    }));
  },

  setRunning(id, queryId, executedSql, mode, unlisten) {
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === id
          ? {
              ...t,
              runState: { phase: "running", startedAt: Date.now(), pid: null, mode },
              activeQueryId: queryId,
              unlisten,
              executedSql,
              results: [],
              activeResult: 0,
              commands: [],
            }
          : t,
      ),
    }));
  },

  startResult(id, resultIndex, cols, pid) {
    set((s) => ({
      tabs: s.tabs.map((t) => {
        if (t.id !== id) return t;
        const results = t.results.slice();
        // Indexes arrive in order, but seed any gap rather than leave holes —
        // `results[i]` is read directly by the grid.
        while (results.length <= resultIndex) results.push({ columns: [], rows: [] });
        results[resultIndex] = { columns: cols, rows: results[resultIndex].rows };
        if (t.runState.phase !== "running") return { ...t, results };
        return { ...t, results, runState: { ...t.runState, pid } };
      }),
    }));
  },

  appendRows(id, resultIndex, rows) {
    set((s) => ({
      tabs: s.tabs.map((t) => {
        if (t.id !== id) return t;
        const results = t.results.slice();
        while (results.length <= resultIndex) results.push({ columns: [], rows: [] });
        const target = results[resultIndex];
        results[resultIndex] = { columns: target.columns, rows: target.rows.concat(rows) };
        return { ...t, results };
      }),
    }));
  },

  setActiveResult(id, resultIndex) {
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, activeResult: resultIndex } : t)),
    }));
  },

  appendCommand(id, cmd) {
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === id ? { ...t, commands: [...t.commands, cmd] } : t,
      ),
    }));
  },

  setDone(id, rowCount, durationMs, cancelled) {
    set((s) => ({
      tabs: s.tabs.map((t) => {
        if (t.id !== id) return t;
        const mode = t.runState.phase === "running" ? t.runState.mode : "buffer";
        if (t.unlisten) {
          try {
            t.unlisten();
          } catch {
            // ignore
          }
        }
        return {
          ...t,
          runState: { phase: "done", rowCount, durationMs, cancelled, mode },
          activeQueryId: null,
          unlisten: null,
        };
      }),
    }));
  },

  setError(id, message) {
    set((s) => ({
      tabs: s.tabs.map((t) => {
        if (t.id !== id) return t;
        if (t.unlisten) {
          try {
            t.unlisten();
          } catch {
            // ignore
          }
        }
        return {
          ...t,
          runState: { phase: "error", message },
          activeQueryId: null,
          unlisten: null,
        };
      }),
    }));
  },

  resetResults(id) {
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === id
          ? {
              ...t,
              results: [],
              activeResult: 0,
              commands: [],
              runState: { phase: "idle" },
            }
          : t,
      ),
    }));
  },
}));
