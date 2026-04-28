import type { ColumnMeta, Row } from "../ipc/query";
import { renderCell } from "./cellRender";

/**
 * RFC 4180-compliant CSV. CRLF line endings, fields containing comma/quote/CR/LF
 * are wrapped in double quotes with internal quotes doubled.
 */
export function rowsToCsv(columns: ColumnMeta[], rows: Row[]): string {
  const out: string[] = [];
  out.push(columns.map((c) => csvField(c.name)).join(","));
  for (const row of rows) {
    const cells = row.map((cell, i) => csvField(renderCell(cell, columns[i].render_kind).text));
    out.push(cells.join(","));
  }
  return out.join("\r\n");
}

function csvField(s: string): string {
  if (s === "") return "";
  if (/[",\r\n]/.test(s)) {
    return `"${s.replace(/"/g, '""')}"`;
  }
  return s;
}

/** Tab-separated, for Excel clipboard. Newlines inside cells are replaced with space. */
export function rowsToTsv(columns: ColumnMeta[], rows: Row[]): string {
  const clean = (s: string) => s.replace(/[\r\n\t]+/g, " ");
  const out: string[] = [];
  out.push(columns.map((c) => clean(c.name)).join("\t"));
  for (const row of rows) {
    out.push(
      row.map((cell, i) => clean(renderCell(cell, columns[i].render_kind).text)).join("\t"),
    );
  }
  return out.join("\n");
}

/** Array of objects keyed by column name. Raw `CellValue` preserved (NULL→null). */
export function rowsToJson(columns: ColumnMeta[], rows: Row[]): string {
  const payload = rows.map((row) => {
    const obj: Record<string, unknown> = {};
    for (let i = 0; i < columns.length; i++) {
      obj[columns[i].name] = row[i];
    }
    return obj;
  });
  return JSON.stringify(payload, null, 2);
}

export function downloadText(name: string, mime: string, content: string) {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
