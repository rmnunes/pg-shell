import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ConnectionSummary,
  EntraSignInEvent,
  ProfileInput,
  ServerInfo,
  TestOutcome,
} from "./types";

export async function connectionsList(): Promise<ConnectionSummary[]> {
  return invoke<ConnectionSummary[]>("connections_list");
}

export async function connectionCreate(
  input: ProfileInput,
  password?: string,
): Promise<ConnectionSummary> {
  return invoke<ConnectionSummary>("connection_create", {
    input,
    password: password ?? null,
  });
}

export async function connectionUpdate(
  id: string,
  input: ProfileInput,
): Promise<ConnectionSummary> {
  return invoke<ConnectionSummary>("connection_update", { id, input });
}

export async function connectionDelete(id: string): Promise<void> {
  await invoke("connection_delete", { id });
}

export async function connectionTest(
  id: string,
  password?: string,
): Promise<TestOutcome> {
  return invoke<TestOutcome>("connection_test", {
    id,
    password: password ?? null,
  });
}

/**
 * Test unsaved connection params. `password` is ignored for Entra profiles,
 * which run a one-off browser sign-in instead.
 */
export async function connectionTestTransient(
  input: ProfileInput,
  password: string | null,
): Promise<TestOutcome> {
  return invoke<TestOutcome>("connection_test_transient", { input, password });
}

export async function connectionConnect(
  id: string,
  password?: string,
): Promise<ServerInfo> {
  return invoke<ServerInfo>("connection_connect", {
    id,
    password: password ?? null,
  });
}

export async function connectionDisconnect(id: string): Promise<void> {
  await invoke("connection_disconnect", { id });
}

export async function connectionPasswordSet(
  id: string,
  password: string,
): Promise<void> {
  await invoke("connection_password_set", { id, password });
}

export async function connectionPasswordClear(id: string): Promise<void> {
  await invoke("connection_password_clear", { id });
}

/** Forget the cached Microsoft Entra sign-in; the next connect opens the browser. */
export async function connectionEntraSignOut(id: string): Promise<void> {
  await invoke("connection_entra_sign_out", { id });
}

/** Subscribe to "a browser sign-in just started" notifications. */
export function onEntraSignIn(
  handler: (event: EntraSignInEvent) => void,
): Promise<UnlistenFn> {
  return listen<EntraSignInEvent>("entra:sign_in", (e) => handler(e.payload));
}
