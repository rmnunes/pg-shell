import { useMemo } from "react";
import { deriveCommandTags } from "./commandTag";

export interface CommandResult {
  index: number;
  rows_affected: number;
}

export interface CommandsPanelProps {
  sql: string;
  commands: CommandResult[];
  /** Duration of the whole batch — displayed in the header. */
  durationMs: number;
  cancelled: boolean;
}

/**
 * Rendered when a query batch produced no result set but completed
 * successfully: `TRUNCATE`, `UPDATE` without RETURNING, `CREATE TABLE`, etc.
 * One row per executed statement, ordered by batch index. Unrecognized tags
 * fall back to "SQL" so an unusual statement still gets a row.
 */
export default function CommandsPanel({
  sql,
  commands,
  durationMs,
  cancelled,
}: CommandsPanelProps) {
  const tags = useMemo(() => deriveCommandTags(sql), [sql]);

  return (
    <div className="commands-panel">
      <div className="commands-header">
        {cancelled ? "Cancelled" : "Completed"} ·{" "}
        <span className="commands-header-time">{durationMs} ms</span>
      </div>
      <div className="commands-list">
        {commands.map((c) => (
          <div key={c.index} className="command-row">
            <span className="command-index">#{c.index + 1}</span>
            <span className="command-tag">{tags[c.index] ?? "SQL"}</span>
            <span className="command-body">{statusLine(tags[c.index], c.rows_affected)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

/**
 * Stringify the rows_affected value to match the shape users expect per
 * command kind. PG's own "Command Complete" messages have variable shape,
 * but `rows_affected` is the single field sqlx exposes reliably.
 */
function statusLine(tag: string | undefined, rowsAffected: number): string {
  if (!tag) return `${rowsAffected.toLocaleString()} row${rowsAffected === 1 ? "" : "s"} affected`;

  // Commands that never report a row count meaningfully.
  if (
    tag === "TRUNCATE" ||
    tag.startsWith("CREATE ") ||
    tag.startsWith("DROP ") ||
    tag.startsWith("ALTER ") ||
    tag === "BEGIN" ||
    tag === "COMMIT" ||
    tag === "ROLLBACK" ||
    tag === "SAVEPOINT" ||
    tag === "SET" ||
    tag === "VACUUM" ||
    tag === "ANALYZE" ||
    tag === "GRANT" ||
    tag === "REVOKE" ||
    tag === "COMMENT"
  ) {
    return "OK";
  }
  return `${rowsAffected.toLocaleString()} row${rowsAffected === 1 ? "" : "s"} affected`;
}
