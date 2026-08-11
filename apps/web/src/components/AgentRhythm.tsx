import { useEffect, useState } from "react";
import {
  fetchAgentProfile,
  profileSentence,
  type AgentProfile,
} from "../lib/agentProfile";

/**
 * One agent's rhythm, inside Agent 360.
 *
 * # WHAT THIS SECTION IS FOR, AND WHY IT IS NOT A CHART PAGE
 *
 * Every other number on this card is a total. A total ranks and does not
 * describe: twenty-six stops in an hour and twenty-six across a month are the
 * same number and different situations. This is the one place the card says
 * which of the two it is looking at, by comparing the agent to ITSELF.
 *
 * So it leads with a SENTENCE and puts the numbers under it. The sentence names
 * both sides of every comparison ("its median day over 90 days is 2; yesterday
 * was 17, which is 8.5x its median day"), because a reader who cannot see the
 * denominator cannot check the claim, and "8.5x" alone is a number to be
 * believed rather than read.
 *
 * # WHAT IT REFUSES TO SAY
 *
 * On `too_new` it reports the counts and refuses the comparison outright,
 * rather than dividing by a baseline it does not have. On `no_data` it says
 * there is nothing to compare against. Neither renders as a calm zero, which
 * would be the same wrong answer the Statistics tab is built to avoid.
 */
export function AgentRhythm({ agentId }: { agentId: string }) {
  const [profile, setProfile] = useState<AgentProfile | null | "loading">("loading");

  useEffect(() => {
    let alive = true;
    setProfile("loading");
    void (async () => {
      const p = await fetchAgentProfile(agentId);
      if (alive) setProfile(p);
    })();
    return () => {
      alive = false;
    };
  }, [agentId]);

  if (profile === "loading") {
    return <Note>loading...</Note>;
  }
  // A failed call, not an answer. The box's own "nothing stored" answer is a
  // real profile with `no_data`, and the two must not render the same.
  if (profile === null) {
    return <Note>the box did not answer, so this agent's rhythm is not shown.</Note>;
  }

  const unusual =
    profile.confidence === "normal" &&
    profile.times_median !== null &&
    profile.times_median >= 3;

  return (
    <div className="flex flex-col gap-2">
      <p
        className="text-[11.5px]"
        style={{ color: unusual ? "var(--fg)" : "var(--dim)", lineHeight: 1.6, margin: 0 }}
      >
        {profileSentence(profile)}
      </p>

      {profile.confidence !== "no_data" && (
        <>
          <Sparkline daily={profile.daily} />
          <div className="flex flex-wrap gap-x-5 gap-y-1">
            <Stat label="window" value={`${profile.days_held}d`} />
            {/* NOT "events". The Delegation row above this card already shows
                an EVENTS count over its own population, and two different
                numbers under one label on one screen is the exact defect this
                console has paid for before (a unit reading 12 agents on one
                card and 16 on another). The label says which population it
                counts, and the window sits beside it. */}
            <Stat
              label="in window"
              value={profile.total.toLocaleString("en-US")}
              title="Every event this agent produced inside the window to the left, which is a different population from the EVENTS count on the Delegation row above."
            />
            <Stat label="median day" value={num(profile.median_day)} />
            <Stat label="yesterday" value={String(profile.latest_full_day)} />
            <Stat
              label="trend"
              value={profile.direction}
              title="The last 7 days against the 7 before them."
            />
            <Stat
              label="busiest day"
              value={`${Math.round(profile.busiest_day_share * 100)}% of window`}
              title="Share of this window's events landing on its single busiest day: a bad afternoon against a bad month."
            />
            {profile.top_type ? (
              <Stat
                label="mostly"
                value={`${profile.top_type} ${Math.round(profile.top_type_share * 100)}%`}
                title="The type that fired most. The same thing over and over and a different thing every time are the same count."
              />
            ) : null}
          </div>
        </>
      )}
    </div>
  );
}

/** Daily totals oldest-first. Deliberately bar-per-day and unlabelled: it is
 * here so the sentence above can be checked against the days it came from, not
 * as a chart to read values off. The busiest day sets the scale, so the shape
 * is always visible whatever the magnitude. */
function Sparkline({ daily }: { daily: number[] }) {
  if (daily.length === 0) return null;
  const peak = Math.max(...daily, 1);
  // Yesterday is the day the sentence judges; today is partial and is drawn
  // faint so nobody reads a half-finished day as a quiet one.
  const judged = daily.length - 2;
  return (
    <div
      className="flex items-end gap-px"
      style={{ height: 28 }}
      title={`${daily.length} day(s), oldest first; peak ${peak}. The last bar is today, still in progress.`}
    >
      {daily.map((n, i) => (
        <div
          key={i}
          style={{
            flex: 1,
            minWidth: 1,
            height: `${Math.max(n === 0 ? 0 : 8, (n / peak) * 100)}%`,
            background:
              i === daily.length - 1
                ? "var(--line-2)"
                : i === judged
                  ? "var(--fg)"
                  : "var(--dim)",
            opacity: i === daily.length - 1 ? 0.5 : 1,
          }}
        />
      ))}
    </div>
  );
}

function Stat({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex items-baseline gap-1.5 min-w-0" title={title}>
      <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
        {label}
      </span>
      <span className="mono tabular text-[11.5px]" style={{ color: "var(--dim)" }}>
        {value}
      </span>
    </div>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return (
    <span className="text-[11px]" style={{ color: "var(--faint)" }}>
      {children}
    </span>
  );
}

function num(n: number): string {
  return Number.isInteger(n) ? String(n) : String(Math.round(n * 100) / 100);
}
