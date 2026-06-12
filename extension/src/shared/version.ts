/** Compare two dotted-numeric versions ("1.1.10" vs "1.1.9").
 *
 * Returns >0 when `a` is newer, <0 when older, 0 when equal. Missing
 * segments count as 0 ("1.1" === "1.1.0"); non-numeric segments compare
 * as 0 — Chrome extension versions are plain dotted integers, so that
 * fallback only matters for malformed input, which must never trigger
 * a reload.
 */
export function compareVersions(a: string, b: string): number {
  const pa = a.split(".");
  const pb = b.split(".");
  const len = Math.max(pa.length, pb.length);
  for (let i = 0; i < len; i += 1) {
    const na = Number.parseInt(pa[i] ?? "0", 10) || 0;
    const nb = Number.parseInt(pb[i] ?? "0", 10) || 0;
    if (na !== nb) return na - nb;
  }
  return 0;
}
