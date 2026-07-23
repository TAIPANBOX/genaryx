import { useCallback, useEffect, useState } from "react";
import type { UiEvent } from "../types";

/**
 * Run Replay's playback clock (docs/PHASE3.md W4, position 5: "a time-window
 * query over the Store + a playback clock ... the mental model of the site
 * sims"). Pure frontend state over one already-fetched, static event list -
 * `Store::events_for_run` (`src-tauri/src/replay.rs`) is a point-in-time
 * query, not a subscription, so there is nothing server-side left to drive a
 * clock; this hook is the whole thing.
 *
 * Model: `revealedCount` is how many of `events` (oldest-first) are
 * currently shown, from 0 (nothing yet) to `events.length` (all shown).
 * While `playing`, a timer advances `revealedCount` by one at a time, paced
 * by the REAL delta between consecutive events' own `ts` (so a burst of
 * events reveals fast and a quiet stretch reveals slowly) scaled by `speed`
 * and clamped to [`MIN_STEP_MS`, `MAX_STEP_MS`] - a real run can span hours,
 * and literally waiting that long would make Play useless, while clamping
 * the floor keeps a dense burst from feeling instant/unwatchable. Playback
 * auto-pauses at the end; `restart` replays from the top.
 */

/** Floor/ceiling on the per-event reveal delay, regardless of the real gap
 * between two consecutive events' timestamps - see this module's doc
 * comment for why neither bound is the literal wall-clock delta. */
const MIN_STEP_MS = 150;
const MAX_STEP_MS = 2_000;

/** The speed multiplier choices `RunReplayView.tsx`'s speed control offers. */
export const REPLAY_SPEEDS: readonly number[] = [0.5, 1, 2, 4, 8];

export interface ReplayClock {
  /** How many of `events` (oldest-first) are currently revealed, 0..N. */
  revealedCount: number;
  playing: boolean;
  speed: number;
  /** `true` once every event has been revealed (and there is at least one) -
   * `RunReplayView.tsx` swaps the Play/Pause control for a Restart one. */
  atEnd: boolean;
  play: () => void;
  pause: () => void;
  toggle: () => void;
  /** Jump straight to `n` revealed events (clamped to `[0, events.length]`)
   * and pause - a manual scrub always stops autoplay, so dragging never
   * fights the timer. */
  seek: (n: number) => void;
  setSpeed: (speed: number) => void;
  /** Back to 0 revealed, paused. */
  reset: () => void;
  /** Back to 0 revealed, then immediately playing - the "watch it again"
   * affordance once `atEnd`. */
  restart: () => void;
}

/** The delay before revealing the event AFTER `fromIndex` (i.e. the step
 * from `fromIndex` to `fromIndex + 1`), from the real gap between their
 * timestamps, scaled by `speed` and clamped. `fromIndex < 0` (nothing
 * revealed yet) or an unparseable/missing pair both fall back to a fixed mid-
 * range delay rather than stalling or firing instantly. */
function stepDelayMs(events: readonly UiEvent[], fromIndex: number, speed: number): number {
  if (fromIndex < 0 || fromIndex + 1 >= events.length) return MIN_STEP_MS;
  const a = Date.parse(events[fromIndex].ts);
  const b = Date.parse(events[fromIndex + 1].ts);
  if (!Number.isFinite(a) || !Number.isFinite(b)) return (MIN_STEP_MS + MAX_STEP_MS) / 2;
  const deltaMs = Math.max(0, b - a);
  return Math.min(MAX_STEP_MS, Math.max(MIN_STEP_MS, deltaMs / speed));
}

export function useReplayClock(events: readonly UiEvent[]): ReplayClock {
  const [revealedCount, setRevealedCount] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);

  // A new event list (a different run was selected) always starts fresh -
  // never carry a stale scrub position or a running timer into a different
  // run's timeline. `events` is a stable reference per fetch (RunReplayView
  // holds it in `useState`, never re-slices/spreads it), so this does not
  // fire on every render.
  useEffect(() => {
    setRevealedCount(0);
    setPlaying(false);
  }, [events]);

  useEffect(() => {
    if (!playing) return;
    if (events.length === 0 || revealedCount >= events.length) {
      setPlaying(false);
      return;
    }
    const delay = stepDelayMs(events, revealedCount - 1, speed);
    const id = window.setTimeout(() => {
      setRevealedCount((n) => Math.min(events.length, n + 1));
    }, delay);
    return () => window.clearTimeout(id);
  }, [playing, revealedCount, events, speed]);

  const atEnd = events.length > 0 && revealedCount >= events.length;

  const play = useCallback(() => {
    setPlaying((was) => was || (events.length > 0 && revealedCount < events.length));
  }, [events.length, revealedCount]);

  const pause = useCallback(() => setPlaying(false), []);

  const toggle = useCallback(() => {
    setPlaying((was) => (was ? false : events.length > 0 && revealedCount < events.length));
  }, [events.length, revealedCount]);

  const seek = useCallback(
    (n: number) => {
      setPlaying(false);
      setRevealedCount(Math.max(0, Math.min(events.length, Math.round(n))));
    },
    [events.length],
  );

  const reset = useCallback(() => {
    setPlaying(false);
    setRevealedCount(0);
  }, []);

  const restart = useCallback(() => {
    setRevealedCount(0);
    setPlaying(true);
  }, []);

  return { revealedCount, playing, speed, atEnd, play, pause, toggle, seek, setSpeed, reset, restart };
}
