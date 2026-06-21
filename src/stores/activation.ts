// First-run AI-activation gate flag (07 §5). Session-scoped on purpose: the gate
// re-appears on the next launch until a provider is actually configured, but once
// the user leaves it this session — by skipping, finishing setup, or "choose
// later" — it stays dismissed so navigating the app never bounces them back to
// /activate (and a just-saved provider can't race the providers query refetch).
import { create } from "zustand";

interface ActivationState {
  /** True once the user has left the first-run AI-activation gate this session. */
  dismissed: boolean;
  dismiss: () => void;
}

export const useActivationStore = create<ActivationState>((set) => ({
  dismissed: false,
  dismiss: () => set({ dismissed: true }),
}));
