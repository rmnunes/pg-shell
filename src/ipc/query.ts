import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type RenderKind =
  | "null"
  | "bool"
  | "int"
  | "float"
  | "numeric"
  | "text"
  | "uuid"
  | "json"
  | "date"
  | "time"
  | "timestamp"
  | "timestamp_tz"
  | "bytea"
  | "array"
  | "unknown";

export interface ColumnMeta {
  name: string;
  type_name: string;
  render_kind: RenderKind;
}

export interface ByteaValue {
  kind: "bytea";
  size: number;
  hex: string;
  truncated: boolean;
}

export type CellValue =
  | null
  | boolean
  | number
  | string
  | ByteaValue
  | CellValue[]
  | { [k: string]: CellValue };

export type Row = CellValue[];

export interface QueryStartEvent {
  query_id: string;
  columns: ColumnMeta[];
  backend_pid: number;
}

export interface QueryRowsEvent {
  query_id: string;
  batch_index: number;
  rows: Row[];
}

export interface QueryCommandEvent {
  query_id: string;
  index: number;
  rows_affected: number;
}

export interface QueryDoneEvent {
  query_id: string;
  total_rows: number;
  total_commands: number;
  duration_ms: number;
  cancelled: boolean;
}

export interface QueryErrorEvent {
  query_id: string;
  message: string;
}

export async function queryExecute(
  profileId: string,
  sql: string,
  queryId: string,
): Promise<void> {
  await invoke("query_execute", {
    profileId,
    sql,
    queryId,
  });
}

export async function queryCancel(queryId: string): Promise<boolean> {
  return invoke<boolean>("query_cancel", { queryId });
}

export interface QueryHandlers {
  onStart?(e: QueryStartEvent): void;
  onRows?(e: QueryRowsEvent): void;
  onCommand?(e: QueryCommandEvent): void;
  onDone?(e: QueryDoneEvent): void;
  onError?(e: QueryErrorEvent): void;
}

/**
 * Subscribe to query lifecycle events, filtering by query_id. Returns an
 * unlisten function that removes all handlers.
 */
export async function subscribeQuery(
  queryId: string,
  handlers: QueryHandlers,
): Promise<UnlistenFn> {
  const unlistens: UnlistenFn[] = [];

  const add = async <E extends { query_id: string }>(
    channel: string,
    handler: ((payload: E) => void) | undefined,
  ) => {
    if (!handler) return;
    const u = await listen<E>(channel, (evt) => {
      if (evt.payload.query_id === queryId) handler(evt.payload);
    });
    unlistens.push(u);
  };

  await add<QueryStartEvent>("query:start", handlers.onStart);
  await add<QueryRowsEvent>("query:rows", handlers.onRows);
  await add<QueryCommandEvent>("query:command", handlers.onCommand);
  await add<QueryDoneEvent>("query:done", handlers.onDone);
  await add<QueryErrorEvent>("query:error", handlers.onError);

  return () => {
    for (const u of unlistens) u();
  };
}
