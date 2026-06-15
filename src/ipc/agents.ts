// Hand-written mirror of the Rust `AgentStatus` DTO (T094, `src-tauri/src/types.rs`
// Module I) until `pnpm gen:types` emits it into `@shared/bindings`. Field shapes
// follow the generated conventions: camelCase.

export type AgentStatusValue = "processing" | "idle" | "offline";

/** Derived Agent presence for one account (T094, F_I2 §4.2). */
export type AgentStatus = {
  accountId: string;
  status: AgentStatusValue;
};
