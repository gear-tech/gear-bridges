import { STATUS_CODES } from 'http';

type Parameters = {
  url: string;
  method?: 'GET' | 'POST' | 'PUT' | 'DELETE';
  parameters?: object;
  isJson?: boolean;
  getErrorMessage?: (response: Response) => Promise<string | undefined>;
};

type FetchWithGuard = {
  <T>(parameters: Parameters & { isJson?: true }): Promise<T>;
  (parameters: Parameters & { isJson?: false }): Promise<Response>;
};

const fetchWithGuard: FetchWithGuard = async <T>({
  url,
  method = 'GET',
  parameters,
  isJson = true,
  getErrorMessage,
}: Parameters) => {
  const headers = { 'Content-Type': 'application/json;charset=utf-8' };
  const body = parameters ? JSON.stringify(parameters) : undefined;

  const response = await fetch(url, { headers, method, body });

  if (!response.ok)
    throw new Error((await getErrorMessage?.(response)) || response.statusText || STATUS_CODES[response.status]);

  return isJson ? (response.json() as T) : response;
};

export { fetchWithGuard };
