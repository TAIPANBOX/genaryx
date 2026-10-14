import { describe, expect, it } from "vitest";

import { FLEET, agentId, mockInvoke } from "./mockPreview";

// The published demo answers every panel from this mock. A model id it names
// is what a visitor reads as the model the console runs on, so an id that has
// retired or is an old generation must not appear in any answer. A retired
// id is a Claude 4.5 model or an OpenAI 4o or 4.1 model.
const RETIRED = /4-5|4o|4\.1/;

// Any model-looking token, bare or inside prose ("cost_per_action:claude-...").
const MODEL_TOKEN = /\b(?:claude|gpt|openai|anthropic|gemini|o\d)[\w./@-]*/gi;

function strings(value: unknown, out: string[] = []): string[] {
  if (typeof value === "string") {
    out.push(value);
  } else if (Array.isArray(value)) {
    for (const item of value) strings(item, out);
  } else if (value && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      out.push(key);
      strings(item, out);
    }
  }
  return out;
}

function modelTokens(answer: unknown): string[] {
  return strings(answer).flatMap((s) => s.match(MODEL_TOKEN) ?? []);
}

describe("the demo's answers name only current models", () => {
  it("the_demo_answers_name_no_retired_model_id", async () => {
    const answers: Array<[string, unknown]> = [];
    const ask = async (command: string, args?: Record<string, unknown>) => {
      answers.push([command, await mockInvoke(command, args)]);
    };

    // The Quality panel: each run's summary, then the scores of every run.
    await ask("quality_list_run_summaries");
    await ask("quality_list_baselines");
    const runs = answers[0][1] as Array<{ run: { id: string } }>;
    for (const row of runs) await ask("quality_run_scores", { run_id: row.run.id });

    // The money panel, its runs, its retained reservations and its gateway
    // price book (credentials_posture carries the price book's model lists).
    await ask("money_runs");
    await ask("money_list_retained");
    await ask("money_gateways");
    await ask("money_units");
    await ask("money_overview");
    await ask("credentials_posture");
    await ask("credentials_gateway_retained_runs");

    // The live stream, the statistics and Felyx's connection.
    await ask("recent_events", { limit: 500 });
    await ask("stats_counts", { window_days: 30 });
    await ask("copilot_status");
    await ask("onboard_status");

    // Every fleet agent's own record and feed, where the model is declared.
    for (const agent of FLEET) {
      await ask("agent_record", { agent_id: agentId(agent) });
      await ask("agent_events", { agent_id: agentId(agent), limit: 50 });
    }

    const seen = answers.flatMap(([, answer]) => modelTokens(answer));
    // The walk must have read model names at all, or it proved nothing.
    expect(seen.length).toBeGreaterThan(0);

    const retired = new Set<string>();
    for (const [command, answer] of answers) {
      for (const token of modelTokens(answer)) {
        if (RETIRED.test(token)) retired.add(`${command}: ${token}`);
      }
    }
    expect([...retired].sort()).toEqual([]);
  });
});
