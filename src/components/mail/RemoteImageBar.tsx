// Remote-image control bar (T029). Shown only when the body has blocked remote
// images and the sender isn't already allow-listed. "This message" does a pure
// in-page DOM swap (data-remote-src → src); "always" persists the sender.
import { useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";

import { useAllowRemoteImages } from "@/ipc/queries/mail";

interface RemoteImageBarProps {
  mailId: string;
  senderEmail: string;
  imagesAllowed: boolean;
  bodyRef: RefObject<HTMLDivElement>;
}

export default function RemoteImageBar({
  mailId,
  senderEmail,
  imagesAllowed,
  bodyRef,
}: RemoteImageBarProps) {
  const { t } = useTranslation();
  const allow = useAllowRemoteImages();
  const [loaded, setLoaded] = useState(false);

  if (imagesAllowed || loaded) {
    return loaded ? (
      <p className="mb-3 font-ui text-[10px] uppercase tracking-wider text-p8">
        {t("images_loaded")}
      </p>
    ) : null;
  }

  const revealInPage = () => {
    const root = bodyRef.current;
    if (root) {
      root.querySelectorAll<HTMLElement>("[data-remote-src]").forEach((el) => {
        const url = el.getAttribute("data-remote-src");
        if (url) {
          el.setAttribute("src", url);
          el.removeAttribute("data-remote-src");
        }
      });
    }
    allow.mutate({ mailId, scope: { type: "thisMessage" } });
    setLoaded(true);
  };

  return (
    <div className="mb-3 flex flex-wrap items-center gap-2 rounded-card bg-p4 px-3 py-2">
      <button
        type="button"
        onClick={revealInPage}
        className="rounded-chip bg-p9 px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-white"
      >
        {t("images_load_this")}
      </button>
      <button
        type="button"
        disabled={allow.isPending}
        onClick={() => allow.mutate({ mailId, scope: { type: "alwaysSender", senderEmail } })}
        className="rounded-chip px-3 py-1.5 font-ui text-xs uppercase tracking-wider text-p8 hover:bg-p3"
      >
        {t("images_load_always")}
      </button>
    </div>
  );
}
