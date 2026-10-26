export function deploymentOrigin(value: unknown, protocols: readonly string[] = ['https:']): string {
  if (typeof value !== 'string' || !value || /[\s*'"\\]/.test(value))
    throw new Error('An exact independently approved origin is required');
  const url = new URL(value);
  if (
    !protocols.includes(url.protocol) ||
    url.username ||
    url.password ||
    value !== url.origin ||
    !/^(?:[a-z0-9.-]+|\[[0-9a-f:]+\])$/.test(url.hostname)
  )
    throw new Error('Only canonical origins without credentials, paths, queries or fragments are allowed');
  return url.origin;
}
