// The generated client resolves every reply as `{ status, data }`; errors are `{ "error": "..." }`.

/** The data of a 200 reply, or undefined. */
export function ok<T>(reply: { status: number; data: T | unknown } | undefined): T | undefined {
  return reply?.status === 200 ? (reply.data as T) : undefined;
}
