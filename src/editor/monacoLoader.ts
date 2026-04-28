/**
 * Point `@monaco-editor/react` at the locally-bundled Monaco package so the
 * app never loads editor code from a CDN. Must run before any editor mounts.
 *
 * Workers use Vite's `?worker` suffix — at build time Vite emits them as
 * separate bundle chunks and the `MonacoEnvironment.getWorker` hook returns
 * the correct one per language. Only the editor worker is wired for SQL
 * because that's the only language we embed; JSON/TS/CSS workers would add
 * megabytes for features we don't need.
 */
import { loader } from "@monaco-editor/react";
import * as monaco from "monaco-editor";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
(self as any).MonacoEnvironment = {
  getWorker(_workerId: string, _label: string) {
    return new EditorWorker();
  },
};

loader.config({ monaco });

/** Call this once during app bootstrap to eagerly resolve the loader. */
export async function initMonaco() {
  await loader.init();
}
