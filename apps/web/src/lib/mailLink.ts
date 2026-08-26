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

/** Which of the three coordinates a link carries. */
export type MailLinkKind = "incident" | "agent" | "owner";

/** What a mail link resolves to. */
export interface MailLink {
  /** Which coordinate this is. */
  kind: MailLinkKind;
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

/**
 * The three path prefixes an alert mail can carry, and what each one is FOR.
 *
 * An operator reading an alert at two in the morning wants three different
 * things depending on what they already know, so the mail offers three
 * coordinates rather than one:
 *
 *   /i/{type}:{subject}  the incident. "What exactly happened."
 *   /a/{agent}           the agent. "Show me this thing, and let me freeze or
 *                        kill it." Opens the agent DETAIL card
 *                        (`AgentDetailCard.tsx`), which is where those two
 *                        controls live. This said "the Agent 360 card, which
 *                        is where those controls live" until 2026-08-03, and
 *                        it was wrong in the way a comment can be: Agent 360
 *                        shows whether an agent is blocked and offers no way
 *                        to block it, so the link landed an operator one
 *                        screen away from the thing the mail had named.
 *   /o/{owner}           the owner. "Who is answerable, and what else are they
 *                        running." The blast radius, when one agent going wrong
 *                        is not the whole story. Opens `UserCard.tsx` on the
 *                        Identity panel; it stopped at the panel alone until
 *                        2026-08-03, which answered the second half of that
 *                        question and not the first.
 *
 * All three are VIEWS. None of them acts: the action happens in the console
 * after a sign-in, and a destructive one after a passkey. A mail that could act
 * would be an unauthenticated capability held by whoever forwards it, and mail
 * gateways prefetch links.
 *
 * `owner` and the delegation chain are DIFFERENT things and the spec keeps them
 * apart (agent-passport SPEC.md sections 4 and 5): `owner` is the required
 * passport field naming who is answerable for the agent existing, a person or a
 * team; `on_behalf_of` carries `user://` principals and says who the agent is
 * acting for right now. Often the same human, not always, and the difference is
 * a different blast radius for a stop. `/o/` is the OWNER, which is who you
 * call (Yurii, 2026-08-02).
 */
const MAIL_LINK_PREFIX = "/i/";
const AGENT_LINK_PREFIX = "/a/";
const OWNER_LINK_PREFIX = "/o/";

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
/**
 * Where the deep link actually arrives from, which is not always the path.
 *
 * The console served by the operator's own box owns its routes, so a mail link
 * lands as `/i/{type}:{subject}` and [`parseMailLink`] reads it straight off
 * `location.pathname`. A STATIC deployment cannot do that: a file server asked
 * for `/i/budget_exhausted:run-42` has no such file and answers 404, so the
 * click dies before any of this code runs. That is not hypothetical, it is
 * where the public demo of this console is published.
 *
 * So the same link is also accepted in the fragment, `#/i/{type}:{subject}`,
 * which every static host serves as the page itself and never sends upstream.
 * One parser, two ways in: the path wins when both are present, because a real
 * console's own route is the more specific statement of intent.
 *
 * Deliberately NOT a query parameter. A fragment is not sent to the server, and
 * this string names an incident and an agent.
 */
export function mailLinkFrom(loc: { pathname: string; hash: string }): MailLink | null {
  const fromPath = parseMailLink(loc.pathname);
  if (fromPath !== null) return fromPath;
  const hash = loc.hash.startsWith("#") ? loc.hash.slice(1) : loc.hash;
  if (hash === "") return null;
  return parseMailLink(hash);
}

/**
 * The address of a thing this console is showing, so an operator can hand it to
 * somebody instead of describing where to click.
 *
 * The exact inverse of [`parseMailLink`], and a round-trip test through that
 * function is what holds the two together. Written as a builder rather than as
 * a template string at the call site because there are two escaping rules here
 * that a caller would get wrong: an agent id carries slashes, which a raw path
 * would read as further segments, and `encodeURIComponent` leaves `/` alone, so
 * it is escaped explicitly.
 *
 * Returns a PATH, not a URL. The origin is the console the reader already has
 * open, and a builder that guessed at one would put this deployment's hostname
 * into a link meant for somebody on a different network. The caller decides
 * whether to prepend `location.origin` or to hand over the fragment form.
 */
export function buildMailLink(kind: MailLinkKind, id: string): string {
  const prefix =
    kind === "incident" ? MAIL_LINK_PREFIX : kind === "agent" ? AGENT_LINK_PREFIX : OWNER_LINK_PREFIX;
  // `encodeURIComponent` escapes almost everything a path would eat and leaves
  // `/` alone, which is exactly the character an `agent://` id is full of.
  return prefix + encodeURIComponent(id).replace(/%2F/gi, "%2F");
}

export function parseMailLink(pathname: string): MailLink | null {
  let kind: MailLinkKind;
  let prefix: string;
  if (pathname.startsWith(MAIL_LINK_PREFIX)) {
    kind = "incident";
    prefix = MAIL_LINK_PREFIX;
  } else if (pathname.startsWith(AGENT_LINK_PREFIX)) {
    kind = "agent";
    prefix = AGENT_LINK_PREFIX;
  } else if (pathname.startsWith(OWNER_LINK_PREFIX)) {
    kind = "owner";
    prefix = OWNER_LINK_PREFIX;
  } else {
    return null;
  }

  let raw = pathname.slice(prefix.length);
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

  // An agent or owner link is the id itself, with no type in front of it.
  if (kind === "agent") {
    return { kind, id, type: "", subject: id, view: "overview" };
  }
  if (kind === "owner") {
    return { kind, id, type: "", subject: id, view: "identity" };
  }

  // `{type}:{subject}` on the FIRST colon only. A subject can contain one:
  // an agent id is `agent://acme.example/biller`, and splitting on every
  // colon would turn it into three pieces and lose the agent.
  const cut = id.indexOf(":");
  const type = cut === -1 ? id : id.slice(0, cut);
  const subject = cut === -1 ? "" : id.slice(cut + 1);

  return { kind, id, type, subject, view: VIEW_BY_TYPE[type] ?? null };
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
  if (link.kind === "agent") {
    return `Opened from an alert about ${link.subject}. Its card is open: freeze or kill it there, not from the mail.`;
  }
  if (link.kind === "owner") {
    return `Opened from an alert about an agent owned by ${link.subject}. This panel is everything they are answerable for.`;
  }
  const about = link.subject === "" ? link.type : `${link.type} on ${link.subject}`;
  if (link.view === null) {
    return `Opened from an alert about ${about}. This console does not know which panel shows that kind of event, so it has not guessed: the id is ${link.id}.`;
  }
  return `Opened from an alert about ${about}.`;
}
