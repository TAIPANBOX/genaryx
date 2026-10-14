import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";

// The sidebar foot is server-rendered here (no DOM in this suite): the
// session badge comes from `useSession`'s first render, which is all the
// demo needs to show. The build flags are set before the modules load, since
// `MOCK` is read once at import time.
async function renderSidebarInDemo(): Promise<string> {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", "1");
  vi.stubEnv("VITE_GENARYX_API", "");
  const { renderToStaticMarkup } = await import("react-dom/server");
  const { AppHeader } = await import("./AppHeader");
  const { PopoverProvider } = await import("../lib/popover");
  return renderToStaticMarkup(
    createElement(
      PopoverProvider,
      null,
      createElement(AppHeader, {
        view: "overview",
        onSelectView: () => {},
        theme: "dark",
        onToggleTheme: () => {},
        policyAlertCount: 0,
        railCollapsed: false,
        onToggleRail: () => {},
      }),
    ),
  );
}

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("AppHeader sidebar foot in the demo", () => {
  it("the_demo_sidebar_foot_shows_the_signed_in_role_and_user", async () => {
    const html = await renderSidebarInDemo();

    expect(html).toContain(">admin<");
    expect(html).toContain(">ops<");
  });

  it("the_demo_sidebar_foot_offers_passkeys_and_connect_this_machine", async () => {
    const html = await renderSidebarInDemo();

    expect(html).toContain(">Passkeys<");
    expect(html).toContain("Connect this machine");
  });
});
