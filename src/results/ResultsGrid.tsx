import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useMemo, useRef, useState } from "react";
import type { CellValue, ColumnMeta, Row } from "../ipc/query";
import JsonViewer from "./JsonViewer";
import { renderCell } from "./cellRender";

export interface ResultsGridProps {
  columns: ColumnMeta[];
  rows: Row[];
}

const ROW_HEIGHT = 24;
const DEFAULT_COL_WIDTH = 160;
const MIN_COL_WIDTH = 60;
const MAX_AUTOFIT_WIDTH = 600;
/** Rows sampled when auto-fitting. Bigger = better fit, smaller = faster. */
const AUTOFIT_SAMPLE = 200;

export default function ResultsGrid({ columns, rows }: ResultsGridProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  /** Per-column-name width overrides persisted across re-renders. Keyed by
   *  column *name* rather than index so re-running the same query doesn't
   *  reset widths even if row counts change. */
  const [overrides, setOverrides] = useState<Record<string, number>>({});
  /** When set, renders the JSON viewer modal with this value. */
  const [viewing, setViewing] = useState<{ title: string; value: CellValue } | null>(null);

  const baseWidths = useMemo(
    () => columns.map((c) => widthForColumn(c.name, c.type_name)),
    [columns],
  );

  const effectiveWidths = useMemo(
    () => columns.map((c, i) => overrides[c.name] ?? baseWidths[i]),
    [columns, baseWidths, overrides],
  );

  const totalWidth = effectiveWidths.reduce((a, b) => a + b, 0) + 48;

  const virt = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 20,
  });

  const setWidth = useCallback((name: string, width: number) => {
    setOverrides((prev) => ({ ...prev, [name]: Math.max(MIN_COL_WIDTH, width) }));
  }, []);

  const autoFit = useCallback(
    (colIdx: number) => {
      const col = columns[colIdx];
      if (!col) return;
      const width = measureAutoFitWidth(col, rows, colIdx);
      setWidth(col.name, width);
    },
    [columns, rows, setWidth],
  );

  return (
    <div className="grid-shell" ref={scrollRef}>
      <div className="grid-inner" style={{ width: totalWidth }}>
        <div className="grid-header">
          <div className="grid-gutter-cell">#</div>
          {columns.map((c, i) => (
            <div
              key={`${c.name}:${i}`}
              className="grid-header-cell"
              style={{ width: effectiveWidths[i] }}
              title={`${c.name} · ${c.type_name}`}
            >
              <div className="grid-header-name">{c.name}</div>
              <div className="grid-header-type">{c.type_name}</div>
              <ResizeHandle
                currentWidth={effectiveWidths[i]}
                onResize={(w) => setWidth(c.name, w)}
                onAutoFit={() => autoFit(i)}
              />
            </div>
          ))}
        </div>
        <div
          className="grid-body"
          style={{ height: virt.getTotalSize(), position: "relative" }}
        >
          {virt.getVirtualItems().map((vr) => {
            const row = rows[vr.index];
            return (
              <div
                key={vr.key}
                className="grid-row"
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  transform: `translateY(${vr.start}px)`,
                  height: ROW_HEIGHT,
                }}
              >
                <div className="grid-gutter-cell">{vr.index + 1}</div>
                {row.map((cell, ci) => {
                  const col = columns[ci];
                  const rendered = renderCell(cell, col.render_kind);
                  const openable =
                    (col.render_kind === "json" || col.render_kind === "array") &&
                    cell !== null;
                  return (
                    <div
                      key={ci}
                      className={`grid-cell${rendered.alignRight ? " align-right" : ""}${openable ? " clickable" : ""}`}
                      style={{ width: effectiveWidths[ci] }}
                      title={openable ? "Click to expand" : rendered.text}
                      onClick={
                        openable
                          ? () =>
                              setViewing({
                                title: `${col.name} · row ${vr.index + 1}`,
                                value: cell,
                              })
                          : undefined
                      }
                    >
                      {rendered.node}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
      {viewing && (
        <JsonViewer
          title={viewing.title}
          value={viewing.value}
          onClose={() => setViewing(null)}
        />
      )}
    </div>
  );
}

interface ResizeHandleProps {
  currentWidth: number;
  onResize: (width: number) => void;
  onAutoFit: () => void;
}

/**
 * Drag zone at the right edge of a header cell. Uses `setPointerCapture` so
 * the native pointer events keep firing on this element even when the cursor
 * leaves the handle — no window listeners, no effect cleanup to go stale
 * mid-drag. All drag state lives in a ref so renders triggered by the
 * width-update don't reset it.
 */
function ResizeHandle({ currentWidth, onResize, onAutoFit }: ResizeHandleProps) {
  const drag = useRef({ startX: 0, startWidth: 0, active: false });

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { startX: e.clientX, startWidth: currentWidth, active: true };
    document.body.classList.add("grid-resizing");
  };

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current.active) return;
    onResize(drag.current.startWidth + (e.clientX - drag.current.startX));
  };

  const endDrag = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current.active) return;
    drag.current.active = false;
    document.body.classList.remove("grid-resizing");
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // capture may already have been released by the browser
    }
  };

  return (
    <div
      className="grid-col-resize"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDoubleClick={(e) => {
        e.stopPropagation();
        onAutoFit();
      }}
      title="Drag to resize · double-click to auto-fit"
    />
  );
}

function widthForColumn(name: string, typeName: string): number {
  const base = Math.max(
    DEFAULT_COL_WIDTH,
    MIN_COL_WIDTH,
    Math.min(320, name.length * 8 + typeName.length * 6 + 40),
  );
  return base;
}

/** Measure pixel width of `text` in the grid body font. One reusable canvas
 *  keeps this cheap even when sampling hundreds of rows. */
let measureCanvas: HTMLCanvasElement | null = null;
function measureText(text: string): number {
  if (!measureCanvas) {
    measureCanvas = document.createElement("canvas");
  }
  const ctx = measureCanvas.getContext("2d");
  if (!ctx) return text.length * 8;
  // Match `.grid-cell` font-family + size (see styles.css).
  ctx.font = "12px 'JetBrains Mono', Consolas, 'Cascadia Mono', monospace";
  return ctx.measureText(text).width;
}

/** Compute a width that fits the header + the widest value in a sample of
 *  `rows`. Returns a number clamped to [MIN_COL_WIDTH, MAX_AUTOFIT_WIDTH]. */
function measureAutoFitWidth(col: ColumnMeta, rows: Row[], colIdx: number): number {
  // Header contributes name (bold 12px) + type (10px) — take the longer of
  // the two rendered, approximated to their body-font measurement + padding.
  const headerPx = Math.max(
    measureText(col.name) * 1.05,
    measureText(col.type_name) * 0.85,
  );
  let widest = headerPx;
  const sample = Math.min(rows.length, AUTOFIT_SAMPLE);
  for (let i = 0; i < sample; i++) {
    const cell = rows[i][colIdx];
    const rendered = renderCell(cell, col.render_kind);
    const w = measureText(rendered.text);
    if (w > widest) widest = w;
  }
  // Add cell padding (8px left + 8px right) and a smidge of safety.
  const px = Math.ceil(widest + 20);
  return Math.max(MIN_COL_WIDTH, Math.min(MAX_AUTOFIT_WIDTH, px));
}
