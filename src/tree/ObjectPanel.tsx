import { useState } from "react";
import FlatView from "./FlatView";
import ObjectTree, { type ContextMenuTarget as _CM } from "./ObjectTree";
import type { TreeNode } from "../ipc/schema";

export type ObjectViewMode = "tree" | "flat";

export interface ObjectPanelProps {
  profileId: string;
  onOpenScript(target: { schema: string; name: string; kind: string }, action: "select" | "insert"): void;
  onViewDefinition(target: { schema: string; name: string; kind: string }): void;
}

/**
 * Houses the object-explorer with a Tree vs Flat view toggle. Tree keeps the
 * canonical hierarchy (schema → categories → relations → columns); Flat
 * collapses to a single searchable list of every relation across every
 * cached schema.
 */
export default function ObjectPanel({ profileId, onOpenScript, onViewDefinition }: ObjectPanelProps) {
  const [mode, setMode] = useState<ObjectViewMode>("tree");

  const treeBridge = (node: TreeNode, action: "select" | "insert") => {
    const [schema, _category, name] = node.path;
    if (!schema || !name) return;
    onOpenScript({ schema, name, kind: node.kind }, action);
  };

  const treeViewDef = (node: TreeNode) => {
    const [schema, _category, name] = node.path;
    const target = schema && name ? name : node.label;
    if (!schema) return;
    onViewDefinition({ schema, name: target, kind: node.kind });
  };

  return (
    <div className="object-panel">
      <div className="view-toggle" role="tablist">
        <button
          role="tab"
          className={mode === "tree" ? "active" : ""}
          onClick={() => setMode("tree")}
          title="Hierarchical: schema → tables/views → columns"
        >
          Tree
        </button>
        <button
          role="tab"
          className={mode === "flat" ? "active" : ""}
          onClick={() => setMode("flat")}
          title="Flat: all relations across all schemas, searchable"
        >
          Flat
        </button>
      </div>
      <div className="object-panel-body">
        {mode === "tree" ? (
          <ObjectTree
            profileId={profileId}
            onOpenScript={treeBridge}
            onViewDefinition={treeViewDef}
          />
        ) : (
          <FlatView
            profileId={profileId}
            onOpenScript={(rel, a) => onOpenScript({ schema: rel.schema, name: rel.name, kind: rel.kind }, a)}
            onViewDefinition={(rel) => onViewDefinition({ schema: rel.schema, name: rel.name, kind: rel.kind })}
          />
        )}
      </div>
    </div>
  );
}
