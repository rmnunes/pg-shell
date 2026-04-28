import type * as Monaco from "monaco-editor";
import { signatureHelp } from "../ipc/signature";

/**
 * SQL signature-help provider.
 *
 * Monaco calls this when the cursor is inside `<name>(` or between `(...)` in
 * expression position. We:
 *   1. Walk backward from the cursor across balanced parens to find the
 *      function name + the enclosing `(`.
 *   2. Count commas between that `(` and the cursor at depth 0 to derive the
 *      active parameter index.
 *   3. Ask the backend for all overloads matching the name.
 *
 * If we can't locate a function-like call, return null — Monaco dismisses the
 * popup gracefully.
 */
export function registerSqlSignatureHelp(
  monaco: typeof Monaco,
  profileId: string,
): Monaco.IDisposable {
  return monaco.languages.registerSignatureHelpProvider("sql", {
    signatureHelpTriggerCharacters: ["(", ","],
    signatureHelpRetriggerCharacters: [","],
    provideSignatureHelp: async (model, position, token) => {
      const doc = model.getValue();
      const offset = model.getOffsetAt(position);
      const ctx = findCallContext(doc, offset);
      if (!ctx) return null;

      let signatures;
      try {
        signatures = await signatureHelp(profileId, ctx.name);
      } catch {
        return null;
      }
      if (token.isCancellationRequested || signatures.length === 0) return null;

      const sigs: Monaco.languages.SignatureInformation[] = signatures.map((s) => ({
        label: s.label,
        documentation: {
          value: `**${s.schema}.${s.name}** → \`${s.result}\``,
          isTrusted: false,
        },
        parameters: s.parameters.map((p) => ({ label: p.label })),
      }));

      // Active signature: prefer the first whose param count >= active index.
      const activeSignature = Math.max(
        0,
        sigs.findIndex((s) => s.parameters.length > ctx.activeParameter),
      );

      return {
        value: {
          signatures: sigs,
          activeSignature: activeSignature < 0 ? 0 : activeSignature,
          activeParameter: ctx.activeParameter,
        },
        dispose: () => undefined,
      };
    },
  });
}

interface CallContext {
  name: string;
  activeParameter: number;
}

/**
 * Walk backward from `cursor` to find the enclosing unbalanced `(`. The
 * identifier immediately preceding that `(` is the function name. Count
 * top-level commas between there and `cursor` to get the active param index.
 *
 * Skips characters inside single-quoted strings. A rough heuristic; good
 * enough for typical interactive typing.
 */
function findCallContext(doc: string, cursor: number): CallContext | null {
  let i = cursor;
  let depth = 0;
  let commas = 0;
  let inString = false;

  while (i > 0) {
    i -= 1;
    const c = doc[i];
    if (inString) {
      if (c === "'") inString = false;
      continue;
    }
    if (c === "'") {
      inString = true;
      continue;
    }
    if (c === ")") {
      depth += 1;
      continue;
    }
    if (c === "(") {
      if (depth === 0) {
        // We're now at the enclosing `(`. Read the identifier immediately
        // before it.
        const nameEnd = i;
        let nameStart = nameEnd;
        while (nameStart > 0) {
          const cc = doc[nameStart - 1];
          if (cc === "_" || cc === "." || /[a-zA-Z0-9]/.test(cc)) {
            nameStart -= 1;
          } else {
            break;
          }
        }
        const raw = doc.slice(nameStart, nameEnd).trim();
        if (!raw) return null;
        return { name: raw, activeParameter: commas };
      }
      depth -= 1;
      continue;
    }
    if (c === "," && depth === 0) {
      commas += 1;
    }
  }
  return null;
}
