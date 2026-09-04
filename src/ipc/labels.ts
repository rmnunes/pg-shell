import type { Profile } from "./types";

/**
 * `user@host:port/db`, or just `host:port/db` when the role is decided at
 * sign-in time (Entra profiles with a blank User).
 */
export function endpointLabel(
  p: Pick<Profile, "user" | "host" | "port" | "database">,
): string {
  const who = p.user ? `${p.user}@` : "";
  return `${who}${p.host}:${p.port}/${p.database}`;
}
