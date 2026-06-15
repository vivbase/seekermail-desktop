// Shared progress bar (T017). Token-styled, RTL-safe (uses logical width only).
interface ProgressBarProps {
  /** 0–100. */
  percent: number;
  /** Optional accent token class (defaults to the work slate). */
  accentClass?: string;
}

export default function ProgressBar({ percent, accentClass = "bg-slate" }: ProgressBarProps) {
  const clamped = Math.max(0, Math.min(100, percent));
  return (
    <div
      className="h-1.5 w-full overflow-hidden rounded-chip bg-p4"
      role="progressbar"
      aria-valuenow={clamped}
    >
      <div className={`h-full ${accentClass}`} style={{ inlineSize: `${clamped}%` }} />
    </div>
  );
}
