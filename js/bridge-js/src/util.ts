export type StatusCb = (status: string, details?: Record<string, string>) => void | Promise<void>;

/** Bounds the awaited operation, without claiming cancellation of an admitted network request. */
export async function withOriginalDeadline<T>(deadline: number, operation: () => Promise<T>): Promise<T> {
  const remaining = deadline - Date.now();
  if (!Number.isSafeInteger(deadline) || remaining <= 0) throw new Error('Original relay deadline expired');
  let timer: ReturnType<typeof setTimeout>;
  const expired = new Promise<T>((_, reject) => {
    timer = setTimeout(() => reject(new Error('Original relay deadline expired')), remaining);
  });
  try { return await Promise.race([operation(), expired]); } finally { clearTimeout(timer!); }
}
