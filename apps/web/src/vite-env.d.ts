/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Base URL of the genaryx-web backend, set only in the web build (e.g.
   * `/api`). Undefined in a bare `vite preview` or the mock build (neither
   * has a backend to reach). Read in `lib/transport.ts`. */
  readonly VITE_GENARYX_API?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
