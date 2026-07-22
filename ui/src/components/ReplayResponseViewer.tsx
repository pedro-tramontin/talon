// Replay response viewer. Wraps the existing
// `ResponseInspector` to show the supplied response.
// Empty state when no response yet (the user hasn't sent).
//
// Phase 5 — §5.4.

import type { ExchangeResponse } from "../types/domain";
import { ResponseInspector } from "./ResponseInspector";

interface Props {
  /** The response to render. `null` shows the empty state. */
  response: ExchangeResponse | null;
}

export function ReplayResponseViewer({ response }: Props) {
  if (!response) {
    return (
      <div
        data-testid="replay-response-viewer-empty"
        className="flex h-full items-center justify-center text-sm text-slate-500"
      >
        Send the request to see the response.
      </div>
    );
  }
  return <ResponseInspector response={response} />;
}
