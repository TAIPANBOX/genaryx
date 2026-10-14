/**
 * The words for whether the Cloud still hears from each gateway. See
 * `crates/api/src/money/gateways.rs` for the five-minute rule and what it
 * cannot tell apart.
 */

import type { Gateways, GatewaySite } from "../moneyTypes";

/** "20 s", "47 min", "3 h", "2 d": how long ago, rounded down. */
export function ageLabel(secs: number): string {
  if (secs <= 0) return "just now";
  if (secs < 60) return `${secs} s`;
  if (secs < 3600) return `${Math.floor(secs / 60)} min`;
  if (secs < 86_400) return `${Math.floor(secs / 3600)} h`;
  return `${Math.floor(secs / 86_400)} d`;
}

/** The line under a site's name. */
export function lastHeardLabel(g: GatewaySite): string {
  return g.state === "silent"
    ? `silent for ${ageLabel(g.last_push_age_secs)}`
    : `heard ${ageLabel(g.last_push_age_secs)}${g.last_push_age_secs > 0 ? " ago" : ""}`;
}

/** The hover on a site's pill. */
export function gatewayTitle(g: GatewaySite, silentAfterSecs: number): string {
  const base = `last push ${g.last_push} (the Cloud's clock) · ${g.pushes.toLocaleString("en-US")} push(es) since ${g.first_push}`;
  const record = g.last_record ? ` · newest call ${g.last_record}` : " · no call reported yet";
  return g.state === "silent"
    ? `The Cloud has not heard from this gateway for more than ${Math.round(silentAfterSecs / 60)} minutes: it stopped pushing calls and heartbeats. Its runs, spend and kills are not moving through the Cloud. ${base}${record}`
    : `${base}${record}`;
}

/** The section's right-hand caption. */
export function gatewaysCaption(data: Gateways): string {
  if (!data.reported) return "not reported by this Cloud";
  const silent = data.gateways.filter((g) => g.state === "silent").length;
  return `${data.gateways.length} site(s) · ${silent} silent`;
}

/** What the section cannot tell apart, printed under the list. */
export const GATEWAYS_CAVEAT =
  "Silent: no push, not even a heartbeat, for more than five minutes. A gateway pushes an empty heartbeat every 30 s by default; one run with TOKENFUSE_CLOUD_HEARTBEAT_SECONDS=0 pushes only when it has calls and reads silent whenever it is idle. Ages use this console's clock against the Cloud's, so a skewed clock moves them.";
