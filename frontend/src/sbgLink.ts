/** Seconds either side of a linked moment that the SBG tab shows. */
export const AROUND_MS = 10_000;

/** Where the SBG tab opens on the frames around `at`, optionally of one connection or console. */
export function sbgLink(at: number, only: { conn?: number; peer?: string } = {}): string {
  const params = new URLSearchParams({ around: String(at) });
  if (only.conn !== undefined) params.set("conn", String(only.conn));
  if (only.peer !== undefined) params.set("peer", only.peer);
  return `#sbg?${params}`;
}

/** The parameters after `?` in the URL's hash. */
export function hashParams(): URLSearchParams {
  const hash = window.location.hash;
  const query = hash.indexOf("?");
  return new URLSearchParams(query < 0 ? "" : hash.slice(query + 1));
}

/** A time as the server's log lines show it, in UTC, to the millisecond. */
export function utcText(ms: number): string {
  return new Date(ms).toISOString();
}

/** The milliseconds of a log line's leading timestamp (`2026-11-09T14:53:15.344834Z`). */
export function logLineTime(line: string): number | undefined {
  const match = /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(\.\d+)?Z/.exec(line);
  if (!match) return undefined;
  const fraction = (match[2] ?? ".0").slice(0, 4);
  const ms = Date.parse(`${match[1]}${fraction}Z`);
  return Number.isNaN(ms) ? undefined : ms;
}

/** The SBG connection a log line names in its `sbg{conn=N ...}` prefix. */
export function logLineConn(line: string): number | undefined {
  const match = /\bsbg\{conn=(\d+)/.exec(line);
  return match ? Number(match[1]) : undefined;
}
