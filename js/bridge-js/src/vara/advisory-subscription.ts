/** Advisory events are never settlement evidence. A malformed event or failed callback closes this subscription. */
export async function subscribeAdvisory<T>(
  register: (dispatch: (decode: () => T | undefined) => void) => Promise<() => void>,
  callback: (value: T) => void | Promise<void>,
  onError: (error: unknown) => void | Promise<void> = (error) => { console.error('Advisory subscription stopped', error); },
): Promise<() => void> {
  let stop: (() => void) | undefined;
  let closed = false;
  const close = () => { closed = true; stop?.(); stop = undefined; };
  const fail = (error: unknown) => {
    if (closed) return;
    close();
    void Promise.resolve().then(() => onError(error)).catch((reportError) => console.error('Advisory error reporter failed', reportError));
  };
  try {
    const unsubscribe = await register((decode) => {
      if (closed) return;
      try {
        const value = decode();
        if (value !== undefined) void Promise.resolve(callback(value)).catch(fail);
      } catch (error) { fail(error); }
    });
    if (closed) unsubscribe(); else stop = unsubscribe;
    return close;
  } catch (error) { fail(error); throw error; }
}
