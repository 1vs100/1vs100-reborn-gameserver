// The generated client resolves every reply as `{ status, data }`; the admin API answers every
// error as `{ "error": "..." }`. These read the two cases.

type Reply = { status: number; data: unknown };

/** The server's message for an error reply, or null for a success. */
export function errorOf(reply: Reply | undefined): string | null {
  if (!reply || reply.status < 400) return null;
  const data = reply.data as { error?: unknown } | null;
  return typeof data?.error === "string" ? data.error : `HTTP ${reply.status}`;
}

/** The data of a 200 reply, or undefined. */
export function ok<T>(reply: { status: number; data: T | unknown } | undefined): T | undefined {
  return reply?.status === 200 ? (reply.data as T) : undefined;
}
