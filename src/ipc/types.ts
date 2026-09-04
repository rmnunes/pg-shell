export type SslMode = "disable" | "prefer" | "require" | "verify_ca" | "verify_full";

/**
 * `password`: classic auth, secret in the OS keychain.
 * `entra_mfa`: Microsoft Entra ID browser sign-in (MFA-capable); the access
 * token is used as the Postgres password, only the refresh token is cached.
 */
export type AuthMethod = "password" | "entra_mfa";

export interface EntraSettings {
  /** Tenant id or verified domain. null → "organizations". */
  tenant: string | null;
  /** Public-client app registration id. null → built-in default (Azure CLI). */
  client_id: string | null;
}

export interface Profile {
  id: string;
  name: string;
  host: string;
  port: number;
  database: string;
  /** Postgres role. Empty on an Entra profile means "the account that signs in". */
  user: string;
  ssl_mode: SslMode;
  app_name: string | null;
  group: string | null;
  auth_method: AuthMethod;
  entra: EntraSettings | null;
}

export interface ProfileInput {
  name: string;
  host: string;
  port: number;
  database: string;
  user: string;
  ssl_mode: SslMode;
  app_name?: string | null;
  group?: string | null;
  auth_method: AuthMethod;
  entra?: EntraSettings | null;
}

export interface ConnectionSummary extends Profile {
  connected: boolean;
  /**
   * A secret for this profile is in the keychain: the password for password
   * auth, a cached sign-in (refresh token) for Entra.
   */
  has_password: boolean;
}

export interface ServerInfo {
  server_version: string;
  current_database: string;
  current_user: string;
}

export interface TestOutcome {
  latency_ms: number;
  server: ServerInfo;
}

export interface AppError {
  kind: string;
  message: string;
}

/** Fired when the backend hands an Entra sign-in off to the system browser. */
export interface EntraSignInEvent {
  /** null for the pre-save "Test" flow in the connection dialog. */
  profile_id: string | null;
  url: string;
}
