/**
 * The other end of the alert mail's one link.
 *
 * `heraldyx` (the notifier that runs beside this console on the operator's own
 * box) mails an alert with exactly one link in it, and that link is a
 * coordinate rather than a control: `https://<console>/i/<type>:<subject>`. It
 * is a GET at a view, it carries no token and no action, because a link that
 * ACTS is an unauthenticated capability held by anyone who sees or forwards
 * the message, and mail gateways prefetch links.
 *
 * The path shape is TokenFuse's own incident id (`"{kind}:{scope}"`, see
 * `tokenfuse/crates/cloud/src/store.rs`), so a link built by the notifier
 * names the same thing the money plane already stores.
 *
 * This module is the console's half: given a pathname, work out what the mail
 * was about and which panel shows it. Kept framework-free (no React, no DOM)
 * like `lib/incidents.ts` and `lib/dashData.ts`, so it is testable as a pure
 * function and `AppShell.tsx` owns the one `window` read.
 */
import type { ViewId } from "./views";

/** What a mail link resolves to. */
export interface MailLink {
  /** The whole id as it appeared in the URL, e.g. `budget_threshold:run-42`. */
  id: string;
  /** The agent-event type, e.g. `budget_threshold`. */
  type: string;
  /** The run or agent the event was about, e.g. `run-42`. Empty for the
   * org-scoped events that carry no subject of their own. */
  subject: string;
  /** Which panel shows this kind of event, or `null` when this build does not
   * recognise the type. */
  view: ViewId | null;
}

/**
 * Which panel owns which event type.
 *
 * Derived from the `source` column of the event-type registry in
 * `agent-passport/SPEC.md` section 6.2, one entry per type that registry
 * lists, because the producing plane is exactly what decides which panel
 * shows it. Unknown types are NOT guessed: see [`parseMailLink`].
 */
const VIEW_BY_TYPE: Readonly<Record<string, ViewId>> = {
  // tokenfuse, the money plane
  budget_threshold: "money",
  budget_exhausted: "money",
  sustained_loop: "money",
  spend_spike: "money",
  fanout_explosion: "money",
  breaker_tripped: "money",
  run_killed: "money",
  dlp_block: "money",
  taint_block: "money",
  mcp_drift: "money",
  identity_mismatch: "money",
  tool_call: "money",
  // wardryx, the policy plane. Approvals live on the Policy panel, which is
  // also where the in-app approval deep link already lands.
  policy_allow: "policy",
  policy_deny: "policy",
  approval_requested: "policy",
  approval_granted: "policy",
  approval_denied: "policy",
  approval_timeout: "policy",
  // idryx
  excessive_privilege: "identity",
  behavior_anomaly: "identity",
  impossible_travel: "identity",
  mfa_fatigue: "identity",
  new_device: "identity",
  blast_radius_change: "identity",
  attestation_missing: "identity",
  // qryx
  crypto_finding: "crypto",
  crypto_drift: "crypto",
  policy_violation: "crypto",
  evidence_signed: "evidence",
  // verdryx
  eval_run: "quality",
  quality_score: "quality",
  quality_drift: "quality",
  // mockryx
  sim_run: "drills",
  sim_finding: "drills",
  blast_radius_measured: "drills",
  // engram
  memory_written: "memory",
  memory_forgotten: "memory",
  reflection_run: "memory",
  contradiction_found: "memory",
};

/** The path prefix the notifier builds its links with. */
const MAIL_LINK_PREFIX = "/i/";

/**
 * Parse a pathname into what the mail was about, or `null` when it is not a
 * mail link at all (every other path in this app, including `/`).
 *
 * An UNKNOWN type resolves with `view: null` rather than being rejected or
 * mapped to a plausible panel. Two reasons, and both are the same reason:
 * a console built before a plane started emitting a type would otherwise
 * either drop the operator's click on the floor or land them somewhere
 * confidently wrong, and this product's whole proposition is that what it
 * shows is what happened. The caller lands on the overview and says which id
 * it could not place.
 */
export function parseMailLink(pathname: string): MailLink | null {
  if (!pathname.startsWith(MAIL_LINK_PREFIX)) return null;

  let raw = pathname.slice(MAIL_LINK_PREFIX.length);
  // A trailing slash is what a browser or a mail client's link rewriter adds,
  // not something the operator did wrong.
  if (raw.endsWith("/")) raw = raw.slice(0, -1);
  if (raw === "") return null;

  let id: string;
  try {
    id = decodeURIComponent(raw);
  } catch {
    // A malformed escape sequence: use it as it arrived rather than throwing
    // on the boot path of the whole console.
    id = raw;
  }

  // `{type}:{subject}` on the FIRST colon only. A subject can contain one:
  // an agent id is `agent://acme.example/biller`, and splitting on every
  // colon would turn it into three pieces and lose the agent.
  const cut = id.indexOf(":");
  const type = cut === -1 ? id : id.slice(0, cut);
  const subject = cut === -1 ? "" : id.slice(cut + 1);

  return { id, type, subject, view: VIEW_BY_TYPE[type] ?? null };
}

/**
 * The one line the console shows after arriving from a mail link, so the
 * operator can see that this IS the thing they were mailed about.
 *
 * Deliberately states the id verbatim. It is what the mail carried, it is
 * what the plane stored, and an operator comparing the two should not have to
 * translate.
 */
export function mailLinkNotice(link: MailLink): string {
  const about = link.subject === "" ? link.type : `${link.type} on ${link.subject}`;
  if (link.view === null) {
    return `Opened from an alert about ${about}. This console does not know which panel shows that kind of event, so it has not guessed: the id is ${link.id}.`;
  }
  return `Opened from an alert about ${about}.`;
}
