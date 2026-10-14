import { simulatedNoticeText } from "../lib/mockPreview";

/**
 * One plain line, shown only in the Live Demo build, saying that what this
 * panel displays is example data and that nothing it offers reaches a box.
 * The real console never renders it: the text is null there, so the component
 * returns nothing and the panel is unchanged.
 */
export function SimulatedNotice() {
  const text = simulatedNoticeText();
  if (text === null) return null;
  return (
    <div
      role="note"
      className="text-[11.5px]"
      style={{ color: "var(--dim)", lineHeight: 1.6, borderLeft: "2px solid var(--sev-medium)", paddingLeft: 8 }}
    >
      {text}
    </div>
  );
}
