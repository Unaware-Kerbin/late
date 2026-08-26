import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

(self as typeof self & { MonacoEnvironment: { getWorker: () => Worker } }).MonacoEnvironment = {
  getWorker() {
    return new EditorWorker();
  },
};
