import type { ByteaValue, CellValue, RenderKind } from "../ipc/query";

export interface RenderedCell {
  /** Text content (used for copy/export). */
  text: string;
  /** React node for display. */
  node: React.ReactNode;
  /** Whether the cell should be right-aligned (numbers). */
  alignRight: boolean;
}

export function renderCell(value: CellValue, kind: RenderKind): RenderedCell {
  if (value === null) {
    return {
      text: "",
      node: <span className="cell-null">NULL</span>,
      alignRight: false,
    };
  }
  if (kind === "bytea" && typeof value === "object" && (value as ByteaValue).kind === "bytea") {
    const b = value as ByteaValue;
    const label = `${b.hex}${b.truncated ? `…  (${b.size} bytes)` : ""}`;
    return {
      text: b.hex,
      node: <span className="cell-bytea" title={`${b.size} bytes`}>{label}</span>,
      alignRight: false,
    };
  }
  if (kind === "json" || kind === "array") {
    const s = JSON.stringify(value);
    return {
      text: s,
      node: <span className="cell-json">{s}</span>,
      alignRight: false,
    };
  }
  if (kind === "bool") {
    return {
      text: String(value),
      node: <span className="cell-bool">{String(value)}</span>,
      alignRight: false,
    };
  }
  if (kind === "int" || kind === "float") {
    return {
      text: String(value),
      node: <>{String(value)}</>,
      alignRight: true,
    };
  }
  if (kind === "numeric") {
    return {
      text: String(value),
      node: <span className="cell-numeric">{String(value)}</span>,
      alignRight: true,
    };
  }
  if (kind === "uuid") {
    return {
      text: String(value),
      node: <span className="cell-uuid">{String(value)}</span>,
      alignRight: false,
    };
  }
  if (
    kind === "date" ||
    kind === "time" ||
    kind === "timestamp" ||
    kind === "timestamp_tz"
  ) {
    return {
      text: String(value),
      node: <span className="cell-temporal">{String(value)}</span>,
      alignRight: false,
    };
  }
  // text / unknown
  return {
    text: typeof value === "string" ? value : JSON.stringify(value),
    node: <>{typeof value === "string" ? value : JSON.stringify(value)}</>,
    alignRight: false,
  };
}
