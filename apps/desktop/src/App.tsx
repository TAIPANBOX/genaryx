import { AppShell } from "./components/AppShell";
import { WebGate } from "./components/WebGate";

export default function App() {
  return (
    <WebGate>
      <AppShell />
    </WebGate>
  );
}
