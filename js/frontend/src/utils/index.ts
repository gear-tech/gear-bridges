import { decodeAddress, encodeAddress, ExtrinsicFailedData } from '@gear-js/api';
import { BaseError } from 'wagmi';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { fetchWithGuard } from './fetch-with-guard';

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
const isNull = (value: unknown): value is null => value === null;
const isString = (value: unknown): value is string => typeof value === 'string';
const isNumeric = (value: string) => /^\d+$/.test(value);

// asserts can't use arrow functions
function definedAssert<T>(value: T, name: string): asserts value is NonNullable<T> {
  if (isUndefined(value) || isNull(value)) throw new Error(`${name} is not defined`);
}

// string is only for cancelled sign and send popup error during useSendProgramTransaction
// reevaluate after @gear-js/react-hooks update
const getErrorMessage = (error: Error | WriteContractErrorType | ExtrinsicFailedData | string) => {
  if (typeof error === 'object' && 'docs' in error) {
    return error.docs || error.method || error.name;
  }

  return typeof error === 'string' ? error : (error as BaseError).shortMessage || error.message;
};

const getTruncatedText = (value: string, prefixLength: number = 6) => {
  if (value.length <= prefixLength) return value;

  return `${value.substring(0, prefixLength)}...${value.slice(-prefixLength)}`;
};

export {
  cx,
  isValidAddress,
  logger,
  asOptionalField,
  isUndefined,
  isNull,
  isString,
  isNumeric,
  getErrorMessage,
  definedAssert,
  fetchWithGuard,
  getTruncatedText,
};
