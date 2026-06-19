// Reusable sender avatar (T037). Renders a fetched avatar image when one is
// supplied; otherwise the sender's email initial on a deterministic dashboard-
// panel color circle. Decorative only (aria-hidden) — the adjacent sender name
// owns the accessible label. Callers pass a font-size via `className` so the same
// component scales from the 36 px list avatar down to the 28 px thread chips.
import { accountColorClass } from "@/lib/accountColor";
import { cn } from "@/lib/cn";
import { senderColorToken, senderInitial } from "@/lib/senderAvatar";

export interface SenderAvatarProps {
  /** Sender email — seeds the deterministic color and the fallback initial. */
  email: string;
  /** Optional display name — used for the initial only when the email is blank. */
  name?: string | null;
  /** Optional fetched avatar image URL. When set, the image replaces the initial. */
  avatarUrl?: string | null;
  /** Pixel diameter of the circle. Defaults to 36. */
  size?: number;
  /** Extra classes (e.g. a `text-*` size override for smaller chips). */
  className?: string;
}

/**
 * Sender / correspondent avatar. Both the color and the initial derive from the
 * *sender*, not the owning account, so a single-account inbox still shows a
 * distinct mark per sender.
 */
export function SenderAvatar({ email, name, avatarUrl, size = 36, className }: SenderAvatarProps) {
  const dimensions = { width: size, height: size };

  if (avatarUrl) {
    return (
      <img
        src={avatarUrl}
        alt=""
        aria-hidden="true"
        loading="lazy"
        style={dimensions}
        className={cn("shrink-0 rounded-avatar object-cover", className)}
      />
    );
  }

  return (
    <div
      aria-hidden="true"
      style={dimensions}
      className={cn(
        "flex shrink-0 items-center justify-center rounded-avatar font-semibold uppercase",
        accountColorClass(senderColorToken(email)),
        className,
      )}
    >
      {senderInitial(email, name)}
    </div>
  );
}
