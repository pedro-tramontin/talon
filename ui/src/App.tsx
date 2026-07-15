import { useEffect, useState } from "react";
import { greet, type Greeting } from "./api";

/**
 * Phase 1 placeholder shell. Shows the Tauri IPC bridge is alive
 * (the `greet` call round-trips) and displays the engine version.
 * Real UI (capture list, replay tabs, fuzz view, ...) lands in Phase 4.
 */
export function App() {
  const [greeting, setGreeting] = useState<Greeting | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    greet()
      .then(setGreeting)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <div className="h-full w-full flex flex-col items-center justify-center gap-4">
      <h1 className="text-4xl font-bold text-accent">Talon</h1>
      {greeting && (
        <p className="text-slate-300">
          {greeting.message}{" "}
          <span className="text-slate-500">v{greeting.version}</span>
        </p>
      )}
      {error && (
        <p className="text-red-400 text-sm">
          Failed to call Rust: {error}
        </p>
      )}
      <p className="text-slate-500 text-sm mt-8">
        v0.1 skeleton · real UI lands in Phase 4
      </p>
    </div>
  );
}
