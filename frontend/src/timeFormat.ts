// XSOR `UTCTime` fields (`startTimeUtc` etc.) and timeline timestamps are UTC ISO strings;
// this converts them for local display.

/** Full UTC ISO datetime (an XSOR `UTCTime` field, e.g. "...Z") -> local wall-clock time. */
export function localizeIsoTime(iso: string): string {
  const parsed = new Date(iso);
  return Number.isNaN(parsed.getTime()) ? iso : parsed.toLocaleTimeString();
}
