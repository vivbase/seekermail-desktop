// First-run AI-activation gate (companion to accountGate.ts, 07 §4). Once at
// least one account exists, the shell stays gated until that account has a real
// AI provider configured — so a freshly installed app routes the user to add a
// provider key and start in Semi-Auto before the agents can act. The gate is
// suppressed for the rest of the session once the user explicitly skips
// (useActivationStore); it returns on the next launch until a key is added.
import { useConfiguredProviders } from "@/ipc/queries/aiProviders";
import { useActivationStore } from "@/stores/activation";

export interface AiActivationGate {
  /** False until the configured-providers query has resolved (avoids a gate flash). */
  ready: boolean;
  /** True when an account exists but no provider is configured and the user has not skipped. */
  needsActivation: boolean;
}

export function useAiActivationGate(): AiActivationGate {
  const { data, isLoading } = useConfiguredProviders();
  const dismissed = useActivationStore((s) => s.dismissed);

  // While the query is in flight we report "not ready" so callers render the
  // shell rather than flashing the gate, then redirect once the answer is known.
  if (isLoading || data === undefined) {
    return { ready: false, needsActivation: false };
  }

  const hasProvider = data.some((p) => p.provider !== "none");
  return { ready: true, needsActivation: !hasProvider && !dismissed };
}
