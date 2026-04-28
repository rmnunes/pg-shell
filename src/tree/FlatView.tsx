import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { schemaFlat, type FlatRelation, type TreeNodeKind } from "../ipc/schema";

export interface FlatViewProps {
  profileId: string;
  onOpenScript(rel: { schema: string; name: string; kind: TreeNodeKind }, action: "select" | "insert"): void;
  onViewDefinition(rel: { schema: string; name: string; kind: TreeNodeKind }): void;
}

interface CtxTarget {
  rel: FlatRelation;
  x: number;
  y: number;
}

/**
 * Flat list of every relation across every cached schema. One scrollable list,
 * one search box. Best for users who know the table name and don't want to
 * navigate the hierarchy.
 */
export default function FlatView({ profileId, onOpenScript, onViewDefinition }: FlatViewProps) {
  const [relations, setRelations] = useState<FlatRelation[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [ctx, setCtx] = useState<CtxTarget | null>(null);

  const load = async () => {
    try {
      setError(null);
      setRelations(await schemaFlat(profileId));
    } catch (e: unknown) {
      const err = e as { message?: string };
      setError(err.message ?? String(e));
    }
  };

  useEffect(() => {
    setRelations(null);
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId]);

  // Auto-refresh after DDL. Filter is preserved; the relation list is
  // re-fetched silently in place.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ profile_id: string }>("schema:invalidated", (evt) => {
      if (evt.payload.profile_id !== profileId) return;
      void load();
    }).then((u) => {
      unlisten = u;
    });
    return () => {
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId]);

  useEffect(() => {
    if (!ctx) return;
    const close = () => setCtx(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [ctx]);

  const filtered = useMemo(() => {
    if (!relations) return [];
    if (!filter) return relations;
    const f = filter.toLowerCase();
    return relations.filter(
      (r) =>
        r.qualified.toLowerCase().includes(f) ||
        r.name.toLowerCase().includes(f) ||
        r.schema.toLowerCase().includes(f),
    );
  }, [relations, filter]);

  const handleContext = (e: ReactMouseEvent, rel: FlatRelation) => {
    e.preventDefault();
    setCtx({ rel, x: e.clientX, y: e.clientY });
  };

  return (
    <div className="tree">
      <div className="sidebar-header">
        <span>Objects · flat</span>
        <button onClick={() => void load()}>↻</button>
      </div>
      <div className="tree-filter">
        <input
          autoFocus
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter schema.table…"
        />
      </div>
      <div className="tree-scroll">
        {error && <div className="tree-error">{error}</div>}
        {relations == null && !error && <div className="tree-hint">Warming schemas…</div>}
        {relations && relations.length === 0 && !error && (
          <div className="tree-hint">
            No relations cached yet. The schema warm-up runs in the background
            after connect; try again in a moment.
          </div>
        )}
        {filtered.map((r) => (
          <div
            key={`${r.schema}/${r.name}`}
            className={`tree-row kind-${r.kind}`}
            style={{ paddingLeft: 10 }}
            onContextMenu={(e) => handleContext(e, r)}
          >
            <span className="tree-icon">{iconFor(r.kind)}</span>
            <span className="tree-label">{r.qualified}</span>
            <span className="tree-detail">{kindLabel(r.kind)}</span>
          </div>
        ))}
      </div>
      {ctx && (
        <div
          className="tree-ctxmenu"
          style={{ top: ctx.y, left: ctx.x }}
          onMouseDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button onClick={() => onOpenScript({ schema: ctx.rel.schema, name: ctx.rel.name, kind: ctx.rel.kind }, "select")}>
            Script as SELECT
          </button>
          {ctx.rel.kind === "table" && (
            <button onClick={() => onOpenScript({ schema: ctx.rel.schema, name: ctx.rel.name, kind: ctx.rel.kind }, "insert")}>
              Script as INSERT
            </button>
          )}
          <button onClick={() => onViewDefinition({ schema: ctx.rel.schema, name: ctx.rel.name, kind: ctx.rel.kind })}>
            View Definition
          </button>
        </div>
      )}
    </div>
  );
}

function iconFor(kind: TreeNodeKind): string {
  switch (kind) {
    case "table":
    case "partitioned_table":
    case "foreign_table":
      return "▤";
    case "view":
    case "materialized_view":
      return "◰";
    case "function":
    case "procedure":
    case "aggregate":
    case "window":
      return "ƒ";
    default:
      return "·";
  }
}

function kindLabel(kind: TreeNodeKind): string {
  switch (kind) {
    case "table":
      return "table";
    case "partitioned_table":
      return "partitioned";
    case "foreign_table":
      return "foreign";
    case "view":
      return "view";
    case "materialized_view":
      return "matview";
    default:
      return "";
  }
}
