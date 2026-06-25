// Keep-alive tab "is-active" context (WB-02). Kept in its own file so component files only
// export components (react-refresh). Children read this to pause work while hidden (18 §10).
import { createContext, useContext } from "react";

export const TabActiveContext = createContext<boolean>(true);

/**
 * Whether the surrounding tab is currently active. Heavy children (virtualized lists,
 * polling effects) should pause work when this is `false` to avoid background churn
 * while kept alive (18 §10, test WT-24).
 */
export function useIsTabActive(): boolean {
  return useContext(TabActiveContext);
}
