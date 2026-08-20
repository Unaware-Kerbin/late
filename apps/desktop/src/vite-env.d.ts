/// <reference types="vite/client" />

interface LateRuntime {
  token?: () => Promise<string>;
}

interface Window {
  lateRuntime?: LateRuntime;
}
