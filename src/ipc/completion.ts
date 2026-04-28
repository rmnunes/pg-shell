import { invoke } from "@tauri-apps/api/core";

export type CompletionKind =
  | "keyword"
  | "snippet"
  | "schema"
  | "table"
  | "view"
  | "materialized_view"
  | "column"
  | "function"
  | "alias";

export interface CompletionItem {
  label: string;
  insert_text: string;
  detail: string | null;
  kind: CompletionKind;
  sort_score: number;
  is_snippet: boolean;
  replace_start: number;
  replace_end: number;
}

export async function completionGet(
  profileId: string,
  doc: string,
  cursorOffset: number,
): Promise<CompletionItem[]> {
  return invoke<CompletionItem[]>("completion_get", {
    profileId,
    doc,
    cursorOffset,
  });
}

/**
 * Record that the user accepted a suggestion. The backend increments its MRU
 * counter so subsequent completion requests rank this identifier higher.
 * Fire-and-forget from the caller's perspective — errors are swallowed.
 */
export async function completionAccept(
  profileId: string,
  kind: CompletionKind,
  identifier: string,
): Promise<void> {
  try {
    await invoke("completion_accept", { profileId, kind, identifier });
  } catch {
    // MRU is a soft signal; failing to record should not surface to the user.
  }
}
