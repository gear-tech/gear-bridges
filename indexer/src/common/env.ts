import * as dotenv from 'dotenv';

dotenv.config();

export const getEnv = (key: string, _default?: string): string => {
  const env = process.env[key] || _default;

  if (!env) {
    throw new Error(`Missing env: ${key}`);
  }

  return env;
};
