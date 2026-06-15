// The only routing gate (07 §4): "≥1 account exists, else redirect to onboarding".
// Module A landed at v0.2, so this now reads the real accounts query. While the
// query is loading we report the shell as reachable to avoid an onboarding flash.
import { useAccounts } from "@/ipc/queries/accounts";

export function useHasAccounts(): boolean {
  const { data, isLoading } = useAccounts();
  return isLoading || (data?.length ?? 0) > 0;
}
