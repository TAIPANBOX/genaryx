import { useEffect, useState } from "react";
import type { ViewId } from "../lib/views";
import { AppHeader } from "./AppHeader";
import { BusExplorer } from "./BusExplorer";
import { MoneyView } from "./MoneyView";
import { OverviewView } from "./OverviewView";
import { PolicyView } from "./PolicyView";

/**
 * App root: owns the theme (persisted to `document.documentElement.dataset`
 * the same way the Bus Explorer's header used to on its own) and the active
 * view, and renders the persistent `AppHeader` plus whichever view is
 * selected. The `.app` class (ambient backdrop + full-height flex column,
 * see `index.css`) now lives here instead of inside `BusExplorer`, since it
 * is a whole-app concern shared by all three views, not a Bus-specific one.
 */
export function AppShell() {
  const [theme, setTheme] = useState<"dark" | "light">("dark");
  const [view, setView] = useState<ViewId>("overview");

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  return (
    <div className="app">
      <AppHeader
        view={view}
        onSelectView={setView}
        theme={theme}
        onToggleTheme={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
      />
      {view === "overview" && <OverviewView />}
      {view === "money" && <MoneyView />}
      {view === "policy" && <PolicyView />}
      {view === "bus" && <BusExplorer />}
    </div>
  );
}
