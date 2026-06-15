// Deterministic geometric Agent avatar (T094, F_I2 §3.2). Pure local SVG — no
// Gravatar, no network, no Canvas (CSP-safe). The same email always renders the
// same pattern + color; the account color token is the background, a contrasting
// token paints the geometry. A 6×6 grid mirrored horizontally gives a stable,
// identicon-like mark.

import { useMemo } from "react";

import type { AccountColorToken } from "@/lib/accountColor";

interface AgentAvatarProps {
  /** Drives the deterministic pattern; the same email is always identical. */
  email: string;
  /** Account color token → background fill (never bare hex). */
  colorToken: AccountColorToken | string;
  /** Pixel size of the square/circle. Defaults to 36. */
  size?: number;
  className?: string;
}

/** djb2-style hash folded to an unsigned 32-bit int (T094 §6). */
function hashEmail(email: string): number {
  let h = 5381;
  for (let i = 0; i < email.length; i += 1) {
    h = ((h << 5) + h + email.charCodeAt(i)) | 0;
  }
  return h >>> 0;
}

/** Mulberry32 — a tiny deterministic PRNG seeded by the email hash. */
function mulberry32(seed: number): () => number {
  let s = seed >>> 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Background fill var for a color token; `team` and unknowns fall back to --p9. */
function bgVar(token: string): string {
  return token === "team" ? "var(--p9)" : `var(--${token})`;
}

/** Foreground (geometry) var: dark on the light `sage` surface, light elsewhere
 *  — mirrors the text choices in `accountColor.ts` so contrast always holds. */
function fgVar(token: string): string {
  return token === "sage" ? "var(--p10)" : "var(--p1)";
}

const GRID = 6;
const CELL = 6; // 36 / 6

export default function AgentAvatar({ email, colorToken, size = 36, className }: AgentAvatarProps) {
  const { shapes, clipId } = useMemo(() => {
    const hash = hashEmail(email);
    const rng = mulberry32(hash);
    const shapeKind = hash % 3; // 0 = triangle, 1 = diamond, 2 = dot
    const fg = fgVar(colorToken);
    const nodes: JSX.Element[] = [];

    // Fill the left three columns, then mirror to the right for symmetry.
    for (let col = 0; col < GRID / 2; col += 1) {
      for (let row = 0; row < GRID; row += 1) {
        if (rng() <= 0.5) continue;
        for (const c of [col, GRID - 1 - col]) {
          const x = c * CELL;
          const y = row * CELL;
          const key = `${c}-${row}`;
          if (shapeKind === 2) {
            nodes.push(
              <circle key={key} cx={x + CELL / 2} cy={y + CELL / 2} r={CELL / 2.6} fill={fg} />,
            );
          } else if (shapeKind === 1) {
            const m = CELL / 2;
            nodes.push(
              <polygon
                key={key}
                points={`${x + m},${y} ${x + CELL},${y + m} ${x + m},${y + CELL} ${x},${y + m}`}
                fill={fg}
              />,
            );
          } else {
            nodes.push(
              <polygon
                key={key}
                points={`${x},${y + CELL} ${x + CELL},${y + CELL} ${x},${y}`}
                fill={fg}
              />,
            );
          }
        }
      }
    }
    return { shapes: nodes, clipId: `sm-avatar-clip-${hash}` };
  }, [email, colorToken]);

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 36 36"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
      className={className}
    >
      <defs>
        <clipPath id={clipId}>
          <circle cx={18} cy={18} r={18} />
        </clipPath>
      </defs>
      <g clipPath={`url(#${clipId})`}>
        <rect width={36} height={36} fill={bgVar(colorToken)} />
        {shapes}
      </g>
    </svg>
  );
}
