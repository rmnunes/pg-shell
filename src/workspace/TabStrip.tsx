import type { QueryTabState } from "./tabs";

export interface TabStripProps {
  tabs: QueryTabState[];
  activeId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  onHistory: () => void;
}

/**
 * Horizontal tab strip above the query editor. Click a tab to activate it,
 * middle-click or the `×` to close. `●` marks dirty (modified since open).
 * A live dot marks a running query in a background tab.
 */
export default function TabStrip({
  tabs,
  activeId,
  onActivate,
  onClose,
  onNew,
  onHistory,
}: TabStripProps) {
  return (
    <div className="tab-strip">
      <div className="tab-strip-tabs">
        {tabs.map((t) => (
          <Tab
            key={t.id}
            tab={t}
            active={t.id === activeId}
            onActivate={() => onActivate(t.id)}
            onClose={() => onClose(t.id)}
          />
        ))}
      </div>
      <button
        className="tab-strip-history"
        onClick={onHistory}
        title="Query history (Ctrl+H)"
      >
        ⧖
      </button>
      <button className="tab-strip-add" onClick={onNew} title="New query tab (Ctrl+T)">
        +
      </button>
    </div>
  );
}

function Tab({
  tab,
  active,
  onActivate,
  onClose,
}: {
  tab: QueryTabState;
  active: boolean;
  onActivate: () => void;
  onClose: () => void;
}) {
  const running = tab.runState.phase === "running";
  const errored = tab.runState.phase === "error";

  return (
    <div
      className={`tab ${active ? "active" : ""}`}
      onClick={onActivate}
      onAuxClick={(e) => {
        // Middle-click closes.
        if (e.button === 1) {
          e.preventDefault();
          onClose();
        }
      }}
      title={tab.title}
    >
      <span
        className={`tab-status ${running ? "running" : errored ? "error" : ""}`}
      >
        {running ? "●" : tab.dirty ? "●" : " "}
      </span>
      <span className="tab-title">{tab.title}</span>
      <button
        className="tab-close"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
        title="Close tab"
      >
        ×
      </button>
    </div>
  );
}
