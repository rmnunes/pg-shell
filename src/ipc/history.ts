import { invoke } from "@tauri-apps/api/core";

export interface HistoryEntry {
  id: number;
  profile_id: string;
  sql: string;
  started_at: number;
  duration_ms: number | null;
  row_count: number | null;
  cancelled: boolean;
  error: string | null;
}

export async function historyList(
  profileId: string,
  search?: string,
  limit?: number,
): Promise<HistoryEntry[]> {
  return invoke<HistoryEntry[]>("history_list", {
    profileId,
    limit: limit ?? null,
    search: search ?? null,
  });
}

export async function historyClear(profileId: string): Promise<number> {
  return invoke<number>("history_clear", { profileId });
}
