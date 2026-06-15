// Humanize a byte count into a compact, human-readable string.
// Precision: 1 decimal place. Rules:
//   < 1 KB      → "< 1 KB"
//   < 1 MB      → "X.X KB"
//   < 1 GB      → "X.X MB"
//   ≥ 1 GB      → "X.X GB"
const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;

export function formatBytes(bytes: number): string {
  if (bytes < KB) return "< 1 KB";
  if (bytes < MB) return `${(bytes / KB).toFixed(1)} KB`;
  if (bytes < GB) return `${(bytes / MB).toFixed(1)} MB`;
  return `${(bytes / GB).toFixed(1)} GB`;
}
