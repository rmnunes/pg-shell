export type SslMode = "disable" | "prefer" | "require" | "verify_ca" | "verify_full";

export interface Profile {
  id: string;
  name: string;
  host: string;
  port: number;
  database: string;
  user: string;
  ssl_mode: SslMode;
  app_name: string | null;
  group: string | null;
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
}

export interface ConnectionSummary extends Profile {
  connected: boolean;
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
