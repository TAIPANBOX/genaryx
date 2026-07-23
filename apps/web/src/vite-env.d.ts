/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Base URL of the genaryx-web backend, set only in the web build (e.g.
   * `/api`). Undefined in the desktop shell (which uses Tauri IPC) and in a
   * bare `vite preview` (which has no backend). Read in `lib/transport.ts`. */
  readonly VITE_GENARYX_API?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
