/// <reference types="vite/client" />

interface LateRuntime {
  token?: () => Promise<string>;
  clipboardRead?: () => Promise<string>;
  clipboardWrite?: (text: string, which?: "clipboard" | "selection") => Promise<boolean>;
}

interface Window {
  lateRuntime?: LateRuntime;
}

declare module "monaco-editor/esm/vs/editor/editor.all";
declare module "monaco-editor/esm/vs/basic-languages/python/python.contribution";
declare module "monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution";
declare module "monaco-editor/esm/vs/basic-languages/ruby/ruby.contribution";

