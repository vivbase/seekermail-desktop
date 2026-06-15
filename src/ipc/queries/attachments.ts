// TanStack Query hooks for attachments (T025/T026). Components consume these,
// never `ipc()` directly (07 §6).
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ipc } from "../client";

export const attachmentKeys = {
  forMail: (mailId: string) => ["attachments", mailId] as const,
  localPath: (attachmentId: string) => ["attachment_path", attachmentId] as const,
};

export function useAttachmentsForMail(mailId: string) {
  return useQuery({
    queryKey: attachmentKeys.forMail(mailId),
    queryFn: () => ipc("get_attachments_for_mail", { mail_id: mailId }),
    enabled: !!mailId,
  });
}

/** `local_path` if downloaded; drives the open-vs-download button. */
export function useAttachmentLocalPath(attachmentId: string) {
  return useQuery({
    queryKey: attachmentKeys.localPath(attachmentId),
    queryFn: () => ipc("get_attachment_local_path", { attachment_id: attachmentId }),
    enabled: !!attachmentId,
    staleTime: Infinity, // local_path is stable; invalidated by attachment:ready
  });
}

export function useDownloadAttachment() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (attachmentId: string) =>
      ipc("download_attachment", { attachment_id: attachmentId }),
    onSuccess: (_d, attachmentId) =>
      void qc.invalidateQueries({ queryKey: attachmentKeys.localPath(attachmentId) }),
  });
}

export function useOpenAttachment() {
  return useMutation({
    mutationFn: (attachmentId: string) => ipc("open_attachment", { attachment_id: attachmentId }),
  });
}

export function useRevealAttachment() {
  return useMutation({
    mutationFn: (attachmentId: string) => ipc("reveal_attachment", { attachment_id: attachmentId }),
  });
}
