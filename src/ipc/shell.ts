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
