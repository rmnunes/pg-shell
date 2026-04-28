import Editor, { type OnMount } from "@monaco-editor/react";
import { useEffect, useImperativeHandle, useRef, forwardRef } from "react";
import type * as Monaco from "monaco-editor";
import { registerSqlCompletion } from "./completionProvider";
import { registerSqlSignatureHelp } from "./signatureProvider";

export interface QueryEditorProps {
  value: string;
  onChange: (value: string) => void;
  /**
   * Called with the SQL text to execute. If the user has a non-empty
   * selection, only that text is passed — otherwise the full buffer.
   */
  onRun: (sql: string) => void;
  readOnly?: boolean;
  /**
   * Profile the completion provider should query against. When this changes,
   * the previous provider is disposed and a new one registered so suggestions
   * follow the active connection.
   */
  profileId?: string;
}

export interface QueryEditorHandle {
  /** Return the current selection text, or the full buffer if no selection. */
  getRunText(): string;
}

/**
 * Monaco SQL editor with:
 *  - F5 / Ctrl+Enter → run (selection if non-empty, full buffer otherwise)
 *  - Per-profile completion provider, disposed & re-registered on profile change
 */
const QueryEditor = forwardRef<QueryEditorHandle, QueryEditorProps>(function QueryEditor(
  { value, onChange, onRun, readOnly, profileId },
  ref,
) {
  const runRef = useRef(onRun);
  runRef.current = onRun;
  const editorRef = useRef<Monaco.editor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<typeof Monaco | null>(null);
  const providerRef = useRef<Monaco.IDisposable | null>(null);
  const signatureRef = useRef<Monaco.IDisposable | null>(null);

  useImperativeHandle(
    ref,
    () => ({
      getRunText: () => (editorRef.current ? currentRunText(editorRef.current) : ""),
    }),
    [],
  );

  const handleMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    editor.addAction({
      id: "pg-shell.run-query",
      label: "Run query (selection if any)",
      keybindings: [monaco.KeyCode.F5, monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
      run: () => runRef.current(currentRunText(editor)),
    });
  };

  useEffect(() => {
    const monaco = monacoRef.current;
    if (!monaco) return;
    providerRef.current?.dispose();
    signatureRef.current?.dispose();
    if (profileId) {
      providerRef.current = registerSqlCompletion(monaco, profileId);
      signatureRef.current = registerSqlSignatureHelp(monaco, profileId);
    } else {
      providerRef.current = null;
      signatureRef.current = null;
    }
    return () => {
      providerRef.current?.dispose();
      signatureRef.current?.dispose();
      providerRef.current = null;
      signatureRef.current = null;
    };
    // monacoRef isn't a state value but its presence must gate registration —
    // re-run after mount when it becomes non-null.
  }, [profileId, monacoRef.current]);

  return (
    <Editor
      language="sql"
      theme="vs-dark"
      value={value}
      onChange={(v) => onChange(v ?? "")}
      onMount={handleMount}
      options={{
        fontSize: 13,
        fontFamily: "'JetBrains Mono', Consolas, 'Cascadia Mono', monospace",
        minimap: { enabled: false },
        scrollBeyondLastLine: false,
        renderLineHighlight: "gutter",
        readOnly,
        automaticLayout: true,
        tabSize: 2,
        wordWrap: "on",
        padding: { top: 6, bottom: 6 },
        lineNumbersMinChars: 3,
      }}
    />
  );
});

export default QueryEditor;

/**
 * Preferred run text: user's selection when non-empty, otherwise the whole
 * buffer.
 */
function currentRunText(editor: Monaco.editor.IStandaloneCodeEditor): string {
  const selection = editor.getSelection();
  if (selection && !selection.isEmpty()) {
    const text = editor.getModel()?.getValueInRange(selection) ?? "";
    if (text.trim().length > 0) return text;
  }
  return editor.getValue();
}
