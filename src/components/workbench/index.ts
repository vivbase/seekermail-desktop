// Workbench (Model S) keep-alive host. See spec 18 §2/§3 and task cards WB-01..WB-09.
export { default as WorkbenchTabHost } from "./WorkbenchTabHost";
export type { WorkbenchTabHostProps } from "./WorkbenchTabHost";
export { default as TabPane } from "./TabPane";
export type { TabPaneProps } from "./TabPane";
export { useIsTabActive } from "./tabActiveContext";
export { default as TabStrip } from "./TabStrip";
export type { TabStripProps } from "./TabStrip";
export { default as Tab } from "./Tab";
export type { TabProps } from "./Tab";
export { default as WorkbenchRoot } from "./WorkbenchRoot";
export type { WorkbenchRootProps, WorkspacePages, WorkspacePageComponent } from "./WorkbenchRoot";
export { useOpenWorkspaceTab } from "./useOpenWorkspaceTab";
export { useWorkbenchShortcuts } from "./useWorkbenchShortcuts";
export type { WorkbenchShortcutOptions } from "./useWorkbenchShortcuts";
export { useCloseTabWithGuard } from "./useCloseTab";
export { default as TabSwitcher } from "./TabSwitcher";
export type { TabSwitcherProps } from "./TabSwitcher";
