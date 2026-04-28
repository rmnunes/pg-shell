import { useCallback, useEffect, useMemo, useState } from "react";
import type { CellValue } from "../ipc/query";

export interface JsonViewerProps {
  title: string;
  value: CellValue;
  onClose(): void;
}

/**
 * Modal viewer for JSON/JSONB and array cell values.
 *
 * ## Expansion model
 *
 * Container nodes (objects/arrays) default to open when their depth is less
 * than `DEFAULT_OPEN_DEPTH`. Explicit user actions (click disclosure, bulk
 * expand/collapse) are stored as overrides in `overridesRef` keyed by the
 * node's JSONPath (`$`, `$.users`, `$[0].name`) — explicit wins over default.
 *
 * ## Interactions
 *  - Click the disclosure or key row: toggle this node.
 *  - Shift+click: toggle this node AND every descendant container.
 *  - Expand all / Collapse all header buttons: set every container.
 *  - Reset button: drop all overrides, back to depth-based default.
 *
 * Keyboard: Esc closes. Ctrl/Cmd+C copies the raw JSON when nothing is
 * selected so casual clicks don't break explicit sub-selection copies.
 */
export default function JsonViewer({ title, value, onClose }: JsonViewerProps) {
  const [mode, setMode] = useState<"tree" | "raw">("tree");
  const [copied, setCopied] = useState(false);
  const [overrides, setOverrides] = useState<Map<string, boolean>>(new Map());

  const raw = useMemo(() => safeStringify(value), [value]);
  const containerPaths = useMemo(() => collectContainerPaths(value, "$"), [value]);

  const isOpen = useCallback(
    (path: string, depth: number): boolean => {
      const override = overrides.get(path);
      return override !== undefined ? override : depth < DEFAULT_OPEN_DEPTH;
    },
    [overrides],
  );

  const toggleOne = useCallback(
    (path: string, depth: number) => {
      setOverrides((prev) => {
        const cur = prev.get(path) ?? depth < DEFAULT_OPEN_DEPTH;
        const next = new Map(prev);
        next.set(path, !cur);
        return next;
      });
    },
    [],
  );

  const toggleDeep = useCallback(
    (path: string, depth: number) => {
      // Compute the new state for the root of the subtree and cascade it to
      // every container beneath so a single shift-click expands/collapses the
      // whole branch in one render.
      const currentOpen = overrides.get(path) ?? depth < DEFAULT_OPEN_DEPTH;
      const target = !currentOpen;
      setOverrides((prev) => {
        const next = new Map(prev);
        for (const p of containerPaths) {
          if (p === path || p.startsWith(`${path}.`) || p.startsWith(`${path}[`)) {
            next.set(p, target);
          }
        }
        return next;
      });
    },
    [overrides, containerPaths],
  );

  const expandAll = useCallback(() => {
    setOverrides(new Map(containerPaths.map((p) => [p, true])));
  }, [containerPaths]);

  const collapseAll = useCallback(() => {
    setOverrides(new Map(containerPaths.map((p) => [p, false])));
  }, [containerPaths]);

  const reset = useCallback(() => {
    setOverrides(new Map());
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
        const sel = window.getSelection();
        if (!sel || sel.toString().length === 0) {
          e.preventDefault();
          void navigator.clipboard.writeText(raw).then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 1400);
          });
        }
      } else if (e.altKey && e.key === "e") {
        e.preventDefault();
        expandAll();
      } else if (e.altKey && e.key === "c") {
        e.preventDefault();
        collapseAll();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [raw, onClose, expandAll, collapseAll]);

  const doCopy = async () => {
    try {
      await navigator.clipboard.writeText(raw);
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      // clipboard may be blocked — noop
    }
  };

  const hasContainers = containerPaths.length > 0;

  return (
    <div className="json-viewer-overlay" onClick={onClose}>
      <div
        className="json-viewer-panel"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={title}
      >
        <div className="json-viewer-header">
          <span className="json-viewer-title" title={title}>
            {title}
          </span>
          <div className="json-viewer-modes" role="tablist">
            <button
              role="tab"
              className={mode === "tree" ? "active" : ""}
              onClick={() => setMode("tree")}
            >
              Tree
            </button>
            <button
              role="tab"
              className={mode === "raw" ? "active" : ""}
              onClick={() => setMode("raw")}
            >
              Raw
            </button>
          </div>
          {mode === "tree" && hasContainers && (
            <>
              <button onClick={expandAll} title="Expand all (Alt+E)">
                Expand all
              </button>
              <button onClick={collapseAll} title="Collapse all (Alt+C)">
                Collapse all
              </button>
              <button
                onClick={reset}
                title="Restore depth-based default"
                disabled={overrides.size === 0}
              >
                Reset
              </button>
            </>
          )}
          <button onClick={doCopy} title="Copy raw JSON (Ctrl+C)">
            {copied ? "Copied" : "Copy"}
          </button>
          <button onClick={onClose} title="Close (Esc)">
            ✕
          </button>
        </div>
        <div className="json-viewer-body">
          {mode === "tree" ? (
            <JsonTree
              value={value}
              depth={0}
              path="$"
              isOpen={isOpen}
              toggleOne={toggleOne}
              toggleDeep={toggleDeep}
            />
          ) : (
            <pre className="json-raw">{raw}</pre>
          )}
        </div>
        <div className="json-viewer-footer">
          <span>
            {mode === "tree"
              ? "Click: toggle · Shift+click: toggle subtree · Alt+E/Alt+C: expand/collapse all"
              : "Ctrl+C: copy"}
          </span>
        </div>
      </div>
    </div>
  );
}

const DEFAULT_OPEN_DEPTH = 2;

interface TreeProps {
  value: CellValue;
  depth: number;
  path: string;
  isOpen: (path: string, depth: number) => boolean;
  toggleOne: (path: string, depth: number) => void;
  toggleDeep: (path: string, depth: number) => void;
  inline?: boolean;
}

function JsonTree(props: TreeProps) {
  const { value, inline } = props;
  if (value === null) return <Leaf kind="null" text="null" inline={inline} />;
  if (typeof value === "boolean") return <Leaf kind="bool" text={String(value)} inline={inline} />;
  if (typeof value === "number") return <Leaf kind="number" text={String(value)} inline={inline} />;
  if (typeof value === "string") return <Leaf kind="string" text={JSON.stringify(value)} inline={inline} />;
  if (Array.isArray(value)) return <ArrayNode items={value} {...props} />;
  if (typeof value === "object") return <ObjectNode entries={Object.entries(value)} {...props} />;
  return <Leaf kind="string" text={String(value)} inline={inline} />;
}

function Leaf({
  kind,
  text,
  inline,
}: {
  kind: "null" | "bool" | "number" | "string";
  text: string;
  inline?: boolean;
}) {
  return <span className={`json-${kind}${inline ? " json-inline" : ""}`}>{text}</span>;
}

function ObjectNode({
  entries,
  depth,
  path,
  isOpen,
  toggleOne,
  toggleDeep,
}: { entries: [string, CellValue][] } & TreeProps) {
  if (entries.length === 0) return <span className="json-punct">{`{}`}</span>;
  const open = isOpen(path, depth);
  const handleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.shiftKey) toggleDeep(path, depth);
    else toggleOne(path, depth);
  };
  return (
    <span className="json-node">
      <button className="json-row-toggle" onClick={handleClick}>
        <span className="json-disclosure">{open ? "▾" : "▸"}</span>
        <span className="json-punct">{"{"}</span>
        {!open && (
          <span className="json-collapsed">
            {entries.length} {entries.length === 1 ? "key" : "keys"}
          </span>
        )}
        {!open && <span className="json-punct">{"}"}</span>}
      </button>
      {open && (
        <>
          <div className="json-children">
            {entries.map(([k, v], i) => (
              <div key={k} className="json-child">
                <span className="json-key">{JSON.stringify(k)}</span>
                <span className="json-punct">: </span>
                <JsonTree
                  value={v}
                  depth={depth + 1}
                  path={childPath(path, k)}
                  isOpen={isOpen}
                  toggleOne={toggleOne}
                  toggleDeep={toggleDeep}
                  inline
                />
                {i < entries.length - 1 && <span className="json-punct">,</span>}
              </div>
            ))}
          </div>
          <span className="json-punct">{"}"}</span>
        </>
      )}
    </span>
  );
}

function ArrayNode({
  items,
  depth,
  path,
  isOpen,
  toggleOne,
  toggleDeep,
}: { items: CellValue[] } & TreeProps) {
  if (items.length === 0) return <span className="json-punct">{"[]"}</span>;
  const open = isOpen(path, depth);
  const handleClick = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.shiftKey) toggleDeep(path, depth);
    else toggleOne(path, depth);
  };
  return (
    <span className="json-node">
      <button className="json-row-toggle" onClick={handleClick}>
        <span className="json-disclosure">{open ? "▾" : "▸"}</span>
        <span className="json-punct">{"["}</span>
        {!open && (
          <span className="json-collapsed">
            {items.length} {items.length === 1 ? "item" : "items"}
          </span>
        )}
        {!open && <span className="json-punct">{"]"}</span>}
      </button>
      {open && (
        <>
          <div className="json-children">
            {items.map((v, i) => (
              <div key={i} className="json-child">
                <span className="json-index">{i}</span>
                <span className="json-punct">: </span>
                <JsonTree
                  value={v}
                  depth={depth + 1}
                  path={`${path}[${i}]`}
                  isOpen={isOpen}
                  toggleOne={toggleOne}
                  toggleDeep={toggleDeep}
                  inline
                />
                {i < items.length - 1 && <span className="json-punct">,</span>}
              </div>
            ))}
          </div>
          <span className="json-punct">{"]"}</span>
        </>
      )}
    </span>
  );
}

/**
 * Walk `value` and collect the JSONPath of every object/array node. Used by
 * Expand/Collapse-all to flip every container in one state write.
 */
function collectContainerPaths(value: CellValue, path: string): string[] {
  const out: string[] = [];
  const visit = (v: CellValue, p: string) => {
    if (v === null || typeof v !== "object") return;
    out.push(p);
    if (Array.isArray(v)) {
      v.forEach((item, i) => visit(item, `${p}[${i}]`));
    } else {
      for (const [k, child] of Object.entries(v)) {
        visit(child, childPath(p, k));
      }
    }
  };
  visit(value, path);
  return out;
}

/** JSONPath-style child accessor. Uses bracket notation when the key is not
 *  a simple identifier so the path stays unambiguous. */
function childPath(parent: string, key: string): string {
  const safe = /^[A-Za-z_][A-Za-z0-9_]*$/.test(key);
  return safe ? `${parent}.${key}` : `${parent}[${JSON.stringify(key)}]`;
}

function safeStringify(v: CellValue): string {
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}
