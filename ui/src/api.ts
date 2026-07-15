// Typed wrapper around the Tauri IPC bridge. v0.1 only has `greet`.
// As we add commands, the types here become the contract between
// Rust and the React app.
//
// The `invoke` import is from `@tauri-apps/api/core` in Tauri 2. Older
// guides (Tauri 1) used `@tauri-apps/api/tauri` — that path is gone.

import { invoke } from "@tauri-apps/api/core";

export interface Greeting {
  message: string;
  version: string;
}

export async function greet(): Promise<Greeting> {
  return await invoke<Greeting>("greet");
}
