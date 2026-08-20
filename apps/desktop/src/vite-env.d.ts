/// <reference types="vite/client" />

interface LateRuntime {
  token?: () => Promise<string>;
  clipboardRead?: () => Promise<string>;
  clipboardWrite?: (text: string, which?: "clipboard" | "selection") => Promise<boolean>;
}

interface Window {
  lateRuntime?: LateRuntime;
}
