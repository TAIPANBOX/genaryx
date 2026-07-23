import { AppShell } from "./components/AppShell";
import { WebGate } from "./components/WebGate";
import { PopoverProvider } from "./lib/popover";

export default function App() {
  return (
    <WebGate>
      <PopoverProvider>
        <AppShell />
      </PopoverProvider>
    </WebGate>
  );
}
