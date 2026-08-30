/// <reference types="vite/client" />

interface LateRuntime {
  token?: () => Promise<string>;
  clipboardRead?: () => Promise<string>;
  clipboardWrite?: (text: string, which?: "clipboard" | "selection") => Promise<boolean>;
  pathForFile?: (file: File) => string;
  isDirectory?: (path: string) => Promise<boolean>;
  updateMeta?: () => Promise<{
    lateVersion: string;
    packaged: boolean;
    platform: string;
    arch: string;
  } | null>;
  checkUpdates?: (mcpCwd?: string) => Promise<{
    lateLocal: string;
    orchLocal: string;
    platform: string;
    arch: string;
    prefer?: string;
    lateRelease: import("./lib/updateSync").GithubRelease | null;
    orchRelease: import("./lib/updateSync").GithubRelease | null;
    lateError?: string | null;
    orchError?: string | null;
  }>;
  applyUpdates?: (payload: {
    confirmed: true;
    which: "late" | "orchestrator" | "both";
    mcpCwd?: string;
  }) => Promise<{
    ok: boolean;
    error?: string;
    steps?: Array<{ app?: string; ok?: boolean; message: string; dest?: string }>;
  }>;
  onUpdateStartup?: (cb: (payload: unknown) => void) => () => void;
  onUpdateMenuCheck?: (cb: () => void) => () => void;
}

interface Window {
  lateRuntime?: LateRuntime;
}

declare module "monaco-editor/esm/vs/editor/editor.all";
declare module "monaco-editor/esm/vs/basic-languages/python/python.contribution";
declare module "monaco-editor/esm/vs/basic-languages/yaml/yaml.contribution";
declare module "monaco-editor/esm/vs/basic-languages/ruby/ruby.contribution";
