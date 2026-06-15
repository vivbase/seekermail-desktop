// SeekerMail brand mark — the single source of truth for the in-app logo.
//
// Faithful inline port of `UI/seekermail logo/assets/seekermail-mark.svg`
// (rocket inside a compass ring). Colors are bound to design-system tokens so
// the mark adapts to light/dark automatically with no second asset:
//   - line-art (compass ring, rocket nose/fins, porthole ring, divider) uses
//     `currentColor`, which resolves to --p10 via the `text-p10` class. --p10
//     flips near-black → near-white in dark mode (tokens.css).
//   - the rocket body / flame / porthole inherit --terra / --amber / --slate,
//     and the porthole fill uses --p1 (lightest surface), which inverts to the
//     darkest surface in dark mode — matching seekermail-mark-dark.svg.
//
// CSS variables are applied through inline `style` (a real CSS context) rather
// than presentation attributes, so substitution is reliable across WebViews.

interface BrandMarkProps {
  /** Rendered width and height in px. Default 40. */
  size?: number;
  /** Accessible label. When omitted the mark is decorative (aria-hidden). */
  title?: string;
  /** Extra classes appended after the default `text-p10` ink color. */
  className?: string;
}

const TERRA = { fill: "var(--terra)" } as const;
const AMBER = { fill: "var(--amber)" } as const;
const SLATE = { fill: "var(--slate)" } as const;
const SURFACE = { fill: "var(--p1)" } as const;

export default function BrandMark({ size = 40, title, className }: BrandMarkProps) {
  const labelled = Boolean(title);
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 100 100"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className ? `text-p10 ${className}` : "text-p10"}
      role={labelled ? "img" : undefined}
      aria-label={labelled ? title : undefined}
      aria-hidden={labelled ? true : undefined}
    >
      {/* Compass ring */}
      <circle cx="50" cy="50" r="46" stroke="currentColor" strokeWidth="2.5" />
      <circle cx="50" cy="50" r="41.5" stroke="currentColor" strokeWidth="0.8" opacity="0.5" />

      {/* Cardinal sparkles — alternating terra / amber */}
      <path d="M24,25 L25.2,26.8 L27,28 L25.2,29.2 L24,31 L22.8,29.2 L21,28 L22.8,26.8 Z" style={TERRA} opacity="0.85" />
      <path d="M78,29.6 L78.96,31.04 L80.4,32 L78.96,32.96 L78,34.4 L77.04,32.96 L75.6,32 L77.04,31.04 Z" style={AMBER} opacity="0.85" />
      <path d="M72,66 L72.8,67.2 L74,68 L72.8,68.8 L72,70 L71.2,68.8 L70,68 L71.2,67.2 Z" style={TERRA} opacity="0.85" />
      <path d="M28,61.6 L28.96,63.04 L30.4,64 L28.96,64.96 L28,66.4 L27.04,64.96 L25.6,64 L27.04,63.04 Z" style={AMBER} opacity="0.85" />

      {/* Exhaust flame */}
      <path d="M45,70 C46,80 50,90 50,90 C50,90 54,80 55,70 Z" style={AMBER} />
      <path d="M47.2,70 C48,77 50,84 50,84 C50,84 52,77 52.8,70 Z" style={TERRA} />

      {/* Rocket body + dark detailing */}
      <path d="M50,12 C61,23 62,33 62,41 L62,64 L38,64 L38,41 C38,33 39,23 50,12 Z" style={TERRA} />
      <path d="M50,12 C56,19 59,27 60,34 L40,34 C41,27 44,19 50,12 Z" fill="currentColor" />
      <path d="M38,52 L27,68 L38,62 Z" fill="currentColor" />
      <path d="M62,52 L73,68 L62,62 Z" fill="currentColor" />
      <path d="M43,64 L57,64 L54,70 L46,70 Z" fill="currentColor" />

      {/* Porthole */}
      <circle cx="50" cy="42" r="7" style={SURFACE} />
      <circle cx="50" cy="42" r="7" stroke="currentColor" strokeWidth="2" />
      <circle cx="50" cy="42" r="2.6" style={SLATE} />

      {/* Hull divider */}
      <line x1="38.5" y1="54" x2="61.5" y2="54" stroke="currentColor" strokeWidth="1.4" opacity="0.6" />
    </svg>
  );
}
