import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * Where the events on screen actually come from, as reported by the Rust
 * `bus_status` command (`src-tauri/src/live.rs`'s `BusMode`).
 *
 * `live` means the console is tailing a real environment's `events.dir`, so
 * everything shown is something a product genuinely emitted. `demo` means no
 * environment exists on this machine and the stream is generated. That
 * distinction has to reach the screen: the two look identical otherwise, and
 * a screenshot of invented traffic passed off as a customer's own is exactly
 * what this product exists to make impossible.
 *
 * `unavailable` is a third, honestly separate state: the bus failed to open
 * at all, so the Bus Explorer is serving bundled mock rows.
 */
export type BusMode =
  | { kind: "live"; env: string; dir: string }
  | { kind: "demo"; dir: string }
  | { kind: "unavailable"; reason: string };

/**
 * Ask the core which mode the bus is in.
 *
 * Returns `null` outside a Tauri runtime (a plain `vite build` or browser
 * preview, where there is no core to ask) and on any invoke failure, so a
 * caller renders no claim at all rather than a wrong one.
 */
export async function fetchBusMode(): Promise<BusMode | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<BusMode>("bus_status");
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("bus_status invoke failed:", err);
    return null;
  }
}
