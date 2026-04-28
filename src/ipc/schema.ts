import { invoke } from "@tauri-apps/api/core";

export type TreeNodeKind =
  | "schema"
  | "category"
  | "table"
  | "view"
  | "materialized_view"
  | "partitioned_table"
  | "foreign_table"
  | "function"
  | "procedure"
  | "aggregate"
  | "window"
  | "column"
  | "sequence"
  | "trigger"
  | "index"
  | "primary_key_index"
  | "unique_index";

export interface TreeNode {
  kind: TreeNodeKind;
  path: string[];
  label: string;
  detail: string | null;
  expandable: boolean;
}

export type ObjectKind =
  | "table"
  | "view"
  | "materialized_view"
  | "function"
  | "procedure"
  | "sequence"
  | "trigger"
  | "index";

export interface FlatRelation {
  schema: string;
  name: string;
  kind: TreeNodeKind;
  qualified: string;
}

export async function schemaFlat(profileId: string): Promise<FlatRelation[]> {
  return invoke<FlatRelation[]>("schema_flat", { profileId });
}

export async function schemaBrowse(
  profileId: string,
  path: string[],
): Promise<TreeNode[]> {
  return invoke<TreeNode[]>("schema_browse", { profileId, path });
}

export async function schemaRefresh(
  profileId: string,
  path?: string[],
): Promise<void> {
  await invoke("schema_refresh", { profileId, path: path ?? null });
}

export async function scriptAsSelect(
  profileId: string,
  schema: string,
  relation: string,
): Promise<string> {
  return invoke<string>("script_as_select", { profileId, schema, relation });
}

export async function scriptAsInsert(
  profileId: string,
  schema: string,
  relation: string,
): Promise<string> {
  return invoke<string>("script_as_insert", { profileId, schema, relation });
}

export async function objectDefinition(
  profileId: string,
  kind: ObjectKind,
  schema: string,
  name: string,
): Promise<string> {
  return invoke<string>("object_definition", { profileId, kind, schema, name });
}
