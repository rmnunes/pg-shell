import type * as Monaco from "monaco-editor";
import {
  completionAccept,
  completionGet,
  type CompletionItem,
  type CompletionKind,
} from "../ipc/completion";

/**
 * Register a SQL completion provider bound to a specific profile. Returns a
 * disposable that must be called when the editor unmounts or the profile
 * changes, otherwise stale providers fight over the same model.
 *
 * Monaco natively debounces provider calls during typing (~200ms) so we don't
 * need our own timer. The provider is async and can return at any time —
 * Monaco cancels via `token` when the user keeps typing.
 */
export function registerSqlCompletion(
  monaco: typeof Monaco,
  profileId: string,
): Monaco.IDisposable {
  return monaco.languages.registerCompletionItemProvider("sql", {
    triggerCharacters: [".", " ", "(", ","],
    /**
     * Monaco calls this when the user accepts a suggestion. We ignore the
     * `suggestion` shape beyond its `label` + `kind` — that's enough to
     * record the MRU hit so the next ranking pass floats it up.
     */
    resolveCompletionItem: (suggestion) => {
      const label = typeof suggestion.label === "string" ? suggestion.label : suggestion.label.label;
      const kind = monacoKindToOurs(monaco, suggestion.kind);
      if (kind) {
        void completionAccept(profileId, kind, label);
      }
      return suggestion;
    },
    provideCompletionItems: async (model, position, _ctx, token) => {
      const doc = model.getValue();
      const offset = model.getOffsetAt(position);
      let items: CompletionItem[];
      try {
        items = await completionGet(profileId, doc, offset);
      } catch {
        return { suggestions: [] };
      }
      if (token.isCancellationRequested) {
        return { suggestions: [] };
      }

      const suggestions: Monaco.languages.CompletionItem[] = items.map((it) => {
        const startPos = model.getPositionAt(it.replace_start);
        const endPos = model.getPositionAt(it.replace_end);
        const range: Monaco.IRange = {
          startLineNumber: startPos.lineNumber,
          startColumn: startPos.column,
          endLineNumber: endPos.lineNumber,
          endColumn: endPos.column,
        };

        // Monaco orders suggestions alphabetically by sortText. Invert our
        // score so the highest-scoring item floats to the top: pad to 6
        // digits so sorting is stable across score magnitudes.
        const inverseScore = Math.max(0, 999999 - it.sort_score)
          .toString()
          .padStart(6, "0");

        return {
          label: it.label,
          kind: kindToMonaco(monaco, it.kind),
          detail: it.detail ?? undefined,
          insertText: it.insert_text,
          insertTextRules: it.is_snippet
            ? monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet
            : undefined,
          range,
          sortText: `${inverseScore}-${it.label}`,
          filterText: it.label,
        };
      });

      return { suggestions };
    },
  });
}

function kindToMonaco(monaco: typeof Monaco, k: CompletionKind): Monaco.languages.CompletionItemKind {
  const K = monaco.languages.CompletionItemKind;
  switch (k) {
    case "keyword":
      return K.Keyword;
    case "snippet":
      return K.Snippet;
    case "schema":
      return K.Module;
    case "table":
      return K.Class;
    case "view":
    case "materialized_view":
      return K.Interface;
    case "column":
      return K.Field;
    case "function":
      return K.Function;
    case "alias":
      return K.Variable;
    default:
      return K.Text;
  }
}

function monacoKindToOurs(
  monaco: typeof Monaco,
  k: Monaco.languages.CompletionItemKind,
): CompletionKind | null {
  const K = monaco.languages.CompletionItemKind;
  switch (k) {
    case K.Keyword:
      return "keyword";
    case K.Snippet:
      return "snippet";
    case K.Module:
      return "schema";
    case K.Class:
      return "table";
    case K.Interface:
      return "view";
    case K.Field:
      return "column";
    case K.Function:
      return "function";
    case K.Variable:
      return "alias";
    default:
      return null;
  }
}
