// Thin shell-command wrappers (open / reveal attachment) via the typed ipc()
// client. All @tauri-apps/api access that isn't a listen() call must be mediated
// through ipc() — never invoke directly in components (07 §6, boundary rule).
import { ipc } from "./client";

/** Open a downloaded attachment with the OS default application. */
export async function openAttachment(attachmentId: string): Promise<void> {
  await ipc("open_attachment", { attachment_id: attachmentId });
}

/** Reveal a downloaded attachment in the system file browser (Finder / Explorer). */
export async function revealAttachment(attachmentId: string): Promise<void> {
  await ipc("reveal_attachment", { attachment_id: attachmentId });
}

/**
 * Open an external link (http / https / mailto / tel) in the OS default browser
 * or mail client. Email-body links are intercepted (lib/externalLinks.ts) and
 * routed here so they never navigate the app's own webview away from the SPA.
 */
export async function openExternalUrl(url: string): Promise<void> {
  await ipc("open_external_url", { url });
}
