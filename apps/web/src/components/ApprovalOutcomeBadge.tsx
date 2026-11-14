import { cssVar } from "../lib/cssVars";
import { approvalOutcome } from "../lib/approvalOutcome";
import type { Approval } from "../policyTypes";

export function ApprovalOutcomeBadge({ approval }: { approval: Pick<Approval, "pending" | "decision"> }) {
  const outcome = approvalOutcome(approval);
  return (
    <span className="badge" style={cssVar("tone", outcome.tone)}>
      {outcome.label}
    </span>
  );
}
