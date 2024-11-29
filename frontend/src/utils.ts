import { decodeAddress, encodeAddress } from '@gear-js/api';
import { z } from 'zod';

const cx = (...args: unknown[]) =>
  args
    .filter((arg) => typeof arg === 'string')
    .join(' ')
    .trim();

const isValidAddress = (address: string) => {
  try {
    encodeAddress(decodeAddress(address));
    return true;
  } catch {
    return false;
  }
};

const DEFAULT_LOGGER_STYLES = 'background-color: #444; color: #bada55; padding: 4px; border-radius: 2px';
const ERROR_LOGGER_STYLES = 'background-color: rgb(186, 73, 73); color: #ccc; padding: 4px; border-radius: 2px';

const logger = {
  info: (name: string, value: string | number | bigint | boolean | null | undefined) =>
    console.log(`%c${name}: ${value}`, DEFAULT_LOGGER_STYLES),

  error: <T extends Error>(name: string, error: T) => {
    console.log(`%c${name}:`, ERROR_LOGGER_STYLES);
    console.error(error);
  },
};

const asOptionalField = <T extends z.ZodTypeAny>(schema: T) => schema.or(z.literal(''));

const isUndefined = (value: unknown): value is undefined => value === undefined;

const isNumeric = (value: string) => /^\d+$/.test(value);

export { cx, isValidAddress, logger, asOptionalField, isUndefined, isNumeric };
