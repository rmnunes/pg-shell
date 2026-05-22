import { useCallback, useEffect, useMemo, useState } from "react";
import type { ConnectionSummary } from "../ipc/types";
import HistoryPanel from "./HistoryPanel";
import QueryTab from "./QueryTab";
import TabStrip, { type TabTarget } from "./TabStrip";
import { useTabStore, type QueryTabState } from "./tabs";

interface Props {
  profileId: string;
  /**
   * Known connection profiles. Used to surface the target server next to each
   * tab title — so the user sees where the SQL will execute.
   */
  connections: ConnectionSummary[];
  /**
   * SQL to open a new tab with when the tree injects a script. Bumped via a
   * version id so the same script can be re-opened.
   */
  injectedSql: { text: string; version: number } | null;
}

/**
 * Tab-aware shell for the query surface. Owns the tab lifecycle (open first
 * tab when a profile activates, open new tab for injected scripts, keyboard
 * shortcuts for new/close).
 */
export default function Workspace({ profileId, connections, injectedSql }: Props) {
  const tabs = useTabStore((s) => s.tabs);
  const activeId = useTabStore((s) => s.activeId);
  const openTab = useTabStore((s) => s.openTab);
  const closeTab = useTabStore((s) => s.closeTab);
  const setActive = useTabStore((s) => s.setActive);
  const [historyOpen, setHistoryOpen] = useState(false);

  // Ensure at least one tab exists for this profile.
  useEffect(() => {
    const hasTabForProfile = tabs.some((t) => t.profileId === profileId);
    if (!hasTabForProfile) {
      openTab(profileId);
    }
  }, [profileId, tabs, openTab]);

  // If the active tab doesn't belong to this profile, activate (or create)
  // one that does. This keeps the editor connection in sync with the
  // connection picker.
  useEffect(() => {
    const active = tabs.find((t) => t.id === activeId);
    if (active && active.profileId === profileId) return;
    const match = tabs.find((t) => t.profileId === profileId);
    if (match) {
      setActive(match.id);
    }
  }, [profileId, tabs, activeId, setActive]);

  // Open a new tab for each fresh injection from the sidebar (script, view
  // definition). Compare by version id rather than text so accepting the same
  // script twice still opens a fresh tab.
  useEffect(() => {
    if (!injectedSql) return;
    openTab(profileId, injectedSql.text);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [injectedSql?.version, profileId]);

  // Keyboard shortcuts: Ctrl+T new tab, Ctrl+W close active, Ctrl+H history.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const key = e.key.toLowerCase();
      if (key === "t") {
        e.preventDefault();
        openTab(profileId);
      } else if (key === "w") {
        if (activeId) {
          e.preventDefault();
          closeTab(activeId);
        }
      } else if (key === "h") {
        e.preventDefault();
        setHistoryOpen(true);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [profileId, activeId, openTab, closeTab]);

  const profileTabs = tabs.filter((t) => t.profileId === profileId);
  const activeTab =
    profileTabs.find((t) => t.id === activeId) ?? profileTabs[profileTabs.length - 1] ?? null;

  const connectionMap = useMemo(() => {
    const m = new Map<string, ConnectionSummary>();
    for (const c of connections) m.set(c.id, c);
    return m;
  }, [connections]);

  const getTarget = useCallback(
    (tab: QueryTabState): TabTarget | null => {
      const c = connectionMap.get(tab.profileId);
      if (!c) return null;
      return {
        label: `${c.name} / ${c.database}`,
        detail: `${c.user}@${c.host}:${c.port}/${c.database}${c.connected ? "" : " (disconnected)"}`,
      };
    },
    [connectionMap],
  );

  return (
    <div className="workspace-shell">
      <TabStrip
        tabs={profileTabs}
        activeId={activeTab?.id ?? null}
        getTarget={getTarget}
        onActivate={setActive}
        onClose={closeTab}
        onNew={() => openTab(profileId)}
        onHistory={() => setHistoryOpen(true)}
      />
      <div className="workspace-tab-body">
        {activeTab ? (
          <QueryTab key={activeTab.id} tab={activeTab} />
        ) : (
          <div className="empty-hint">
            <h2>No tabs open</h2>
            <p>Press + above or Ctrl+T to start a new query.</p>
          </div>
        )}
      </div>
      {historyOpen && (
        <HistoryPanel
          profileId={profileId}
          onClose={() => setHistoryOpen(false)}
          onOpen={(sql) => {
            openTab(profileId, sql);
            setHistoryOpen(false);
          }}
        />
      )}
    </div>
  );
}
