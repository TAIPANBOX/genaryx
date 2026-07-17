import { isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { cssVar } from "../lib/cssVars";
import {
  checkSshReachable,
  describeRemoteError,
  readRemoteFile,
  startRemoteTail,
  stopRemoteTail,
} from "../lib/remote";
import type { RemoteError, RemoteFile, RemoteStatus, RemoteTailEnded, RemoteTailLine, TailStatus } from "../remoteTypes";

const FIELD_STYLE = {
  background: "var(--panel)",
  border: "1px solid var(--line-2)",
  borderRadius: 8,
  padding: "6px 10px",
  fontSize: 12,
  color: "var(--fg)",
} as const;

/** Tauri event names `remote::commands` emits - mirrors
 * `remote::commands::{TAIL_LINE_EVENT,TAIL_ENDED_EVENT}` exactly. */
const TAIL_LINE_EVENT = "remote:tail-line";
const TAIL_ENDED_EVENT = "remote:tail-ended";

/** How many recent tail lines stay on screen - a display cap, not a network
 * one (the backend streams every line; this just bounds the DOM). */
const MAX_DISPLAY_LINES = 500;

type ReachableState = "idle" | "checking" | "ok" | { error: RemoteError };

/**
 * Section 4 (docs/PHASE4.md W4 position 4): SSH ops over the pinned target -
 * "Check reachable", "Read remote descriptor", and a live remote log tail
 * streamed over the `remote:tail-line`/`remote:tail-ended` Tauri events (see
 * `remote::commands`'s module doc for the backend side of this stream).
 */
export function RemoteSshOps({
  hasEnvironment,
  tail,
  onStatusChange,
}: {
  hasEnvironment: boolean;
  tail: TailStatus | null;
  onStatusChange: (status: RemoteStatus) => void;
}) {
  // ---- check reachable ----
  const [reachable, setReachable] = useState<ReachableState>("idle");

  const onCheckReachable = useCallback(async () => {
    if (!hasEnvironment) return;
    setReachable("checking");
    try {
      await checkSshReachable();
      setReachable("ok");
    } catch (err) {
      setReachable({ error: err as RemoteError });
    }
  }, [hasEnvironment]);

  // ---- read remote file ----
  const [readPath, setReadPath] = useState("");
  const [reading, setReading] = useState(false);
  const [readResult, setReadResult] = useState<RemoteFile | null>(null);
  const [readError, setReadError] = useState<RemoteError | null>(null);

  const onRead = useCallback(async () => {
    if (!hasEnvironment || readPath.trim().length === 0 || reading) return;
    setReading(true);
    setReadError(null);
    try {
      const file = await readRemoteFile(readPath);
      setReadResult(file);
    } catch (err) {
      setReadError(err as RemoteError);
    } finally {
      setReading(false);
    }
  }, [hasEnvironment, readPath, reading]);

  // ---- remote tail ----
  const [tailPath, setTailPath] = useState("");
  const [fromOffset, setFromOffset] = useState(0);
  const [tailBusy, setTailBusy] = useState(false);
  const [tailError, setTailError] = useState<RemoteError | null>(null);
  const [lines, setLines] = useState<string[]>([]);
  const [endedReason, setEndedReason] = useState<string | null>(null);
  const logRef = useRef<HTMLDivElement | null>(null);

  const running = tail?.running === true;

  const onStartTail = useCallback(async () => {
    if (!hasEnvironment || tailPath.trim().length === 0 || tailBusy) return;
    setTailBusy(true);
    setTailError(null);
    setEndedReason(null);
    setLines([]);
    try {
      const status = await startRemoteTail(tailPath, fromOffset);
      onStatusChange(status);
    } catch (err) {
      setTailError(err as RemoteError);
    } finally {
      setTailBusy(false);
    }
  }, [hasEnvironment, tailPath, fromOffset, tailBusy, onStatusChange]);

  const onStopTail = useCallback(async () => {
    if (tailBusy) return;
    setTailBusy(true);
    setTailError(null);
    try {
      const status = await stopRemoteTail();
      onStatusChange(status);
    } catch (err) {
      setTailError(err as RemoteError);
    } finally {
      setTailBusy(false);
    }
  }, [tailBusy, onStatusChange]);

  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    let unlistenLine: (() => void) | undefined;
    let unlistenEnded: (() => void) | undefined;

    listen<RemoteTailLine>(TAIL_LINE_EVENT, (event) => {
      setLines((prev) => [...prev, event.payload.line].slice(-MAX_DISPLAY_LINES));
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlistenLine = fn;
      })
      .catch((err: unknown) => {
        // eslint-disable-next-line no-console
        console.error(`listen(${TAIL_LINE_EVENT}) failed:`, err);
      });

    listen<RemoteTailEnded>(TAIL_ENDED_EVENT, (event) => {
      setEndedReason(event.payload.reason);
    })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlistenEnded = fn;
      })
      .catch((err: unknown) => {
        // eslint-disable-next-line no-console
        console.error(`listen(${TAIL_ENDED_EVENT}) failed:`, err);
      });

    return () => {
      cancelled = true;
      unlistenLine?.();
      unlistenEnded?.();
    };
  }, []);

  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight });
  }, [lines]);

  // Stop the tail when this section unmounts (navigating away from Remote) -
  // never leave a live ssh child running unattended (mirrors the WgTunnel/
  // SshClient Drop-based teardown philosophy this whole panel follows).
  useEffect(() => {
    return () => {
      void stopRemoteTail().catch(() => {
        // best-effort - the component is already gone, nothing to show
      });
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex flex-col gap-4">
      <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <div className="flex items-center gap-3 flex-wrap">
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
            onClick={() => void onCheckReachable()}
            disabled={!hasEnvironment || reachable === "checking"}
          >
            {reachable === "checking" ? "Checking..." : "Check reachable"}
          </button>
          {reachable === "ok" && (
            <span className="chip" style={cssVar("dot", "var(--sev-low)")}>
              <span className="dot" aria-hidden="true" />
              reachable, host key pinned, auth OK
            </span>
          )}
          {typeof reachable === "object" && (
            <span className="mono text-[11.5px]" style={{ color: "var(--sev-high)" }}>
              {describeRemoteError(reachable.error)}
            </span>
          )}
        </div>
      </div>

      <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11px]" style={{ color: "var(--dim)" }}>
          read remote descriptor
        </span>
        <div className="flex items-center gap-2 flex-wrap">
          <input
            className="mono flex-1"
            style={{ ...FIELD_STYLE, minWidth: 220 }}
            value={readPath}
            onChange={(e) => setReadPath(e.target.value)}
            placeholder="~/.taipan/environments/<name>.json"
            spellCheck={false}
          />
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
            onClick={() => void onRead()}
            disabled={!hasEnvironment || reading || readPath.trim().length === 0}
          >
            {reading ? "Reading..." : "Read"}
          </button>
        </div>
        {readError && (
          <span className="mono text-[11.5px]" style={{ color: "var(--sev-high)" }}>
            {describeRemoteError(readError)}
          </span>
        )}
        {readResult && (
          <div className="flex flex-col gap-1">
            {!readResult.valid_utf8 && (
              <span className="mono text-[10.5px]" style={{ color: "var(--sev-medium)" }}>
                not valid UTF-8 ({readResult.size_bytes} bytes) - shown as a lossy decode below
              </span>
            )}
            <pre className="json-pre" style={{ maxHeight: 240 }}>
              {readResult.content}
            </pre>
          </div>
        )}
      </div>

      <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
        <span className="text-[11px]" style={{ color: "var(--dim)" }}>
          remote log tail
        </span>
        <div className="flex items-center gap-2 flex-wrap">
          <input
            className="mono flex-1"
            style={{ ...FIELD_STYLE, minWidth: 200 }}
            value={tailPath}
            onChange={(e) => setTailPath(e.target.value)}
            placeholder="/var/log/taipan/gateway.log"
            spellCheck={false}
            disabled={running}
          />
          <span className="text-[11px] shrink-0" style={{ color: "var(--dim)" }}>
            from offset
          </span>
          <input
            className="mono"
            style={{ ...FIELD_STYLE, width: 110 }}
            type="number"
            min={0}
            value={fromOffset}
            onChange={(e) => setFromOffset(Number(e.target.value))}
            disabled={running}
          />
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
            onClick={() => void onStartTail()}
            disabled={!hasEnvironment || tailBusy || tailPath.trim().length === 0 || running}
          >
            {tailBusy && !running ? "Starting..." : "Start tail"}
          </button>
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
            onClick={() => void onStopTail()}
            disabled={tailBusy || !running}
          >
            Stop
          </button>
          {running && (
            <span className="chip" style={cssVar("dot", "var(--sev-low)")}>
              <span className="dot" aria-hidden="true" />
              tailing {tail?.path}
            </span>
          )}
        </div>
        {tailError && (
          <span className="mono text-[11.5px]" style={{ color: "var(--sev-high)" }}>
            {describeRemoteError(tailError)}
          </span>
        )}
        {endedReason && !running && (
          <span className="mono text-[10.5px]" style={{ color: "var(--faint)" }}>
            stream ended: {endedReason}
          </span>
        )}
        <div
          ref={logRef}
          className="mono thin-scroll"
          style={{
            background: "var(--panel)",
            border: "1px solid var(--line)",
            borderRadius: 8,
            padding: "8px 10px",
            fontSize: 11,
            lineHeight: 1.6,
            color: "var(--dim)",
            height: 200,
            overflowY: "auto",
            whiteSpace: "pre-wrap",
          }}
        >
          {lines.length === 0 ? (
            <span style={{ color: "var(--faint)" }}>no lines yet - start a tail to see remote log output here.</span>
          ) : (
            lines.map((line, i) => <div key={i}>{line}</div>)
          )}
        </div>
      </div>
    </div>
  );
}
