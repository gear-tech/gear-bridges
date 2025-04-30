import { decodeAddress, encodeAddress, ExtrinsicFailedData, HexString } from '@gear-js/api';
import { BaseError } from 'wagmi';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';

import { TOKEN_SVG, WRAPPED_VARA_CONTRACT_ADDRESS, ETH_WRAPPED_ETH_CONTRACT_ADDRESS } from './consts';

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
const isNumeric = (value: string) => /^\d+$/.test(value);

const getTokenSVG = (address: HexString) => TOKEN_SVG[address] || TokenPlaceholderSVG;

// string is only for cancelled sign and send popup error during useSendProgramTransaction
// reevaluate after @gear-js/react-hooks update
const getErrorMessage = (error: Error | WriteContractErrorType | ExtrinsicFailedData | string) => {
  if (typeof error === 'object' && 'docs' in error) {
    return error.docs || error.method || error.name;
  }

  return typeof error === 'string' ? error : (error as BaseError).shortMessage || error.message;
};

const isNativeToken = (address: HexString) =>
  [WRAPPED_VARA_CONTRACT_ADDRESS, ETH_WRAPPED_ETH_CONTRACT_ADDRESS].includes(address);

// asserts can't use arrow functions
function definedAssert<T>(value: T, name: string): asserts value is NonNullable<T> {
  if (isUndefined(value) || isNull(value)) throw new Error(`${name} is not defined`);
}

const fetchWithGuard = async <T>({
  url,
  method,
  parameters,
}: {
  url: string;
  method?: 'GET' | 'POST' | 'PUT' | 'DELETE';
  parameters?: object;
}) => {
  const headers = { 'Content-Type': 'application/json;charset=utf-8' };
  const body = parameters ? JSON.stringify(parameters) : undefined;

  const response = await fetch(url, { headers, method, body });

  if (!response.ok) {
    if (response.statusText) throw new Error(`Failed: ${response.statusText}`);

    const result = (await response.json().catch(() => {})) as unknown;

    if (result !== null && typeof result === 'object' && 'error' in result) {
      const errorMessage = result.error as string;

      throw new Error(`Failed: ${errorMessage}`);
    }

    throw new Error(`Failed: ${response.status}`);
  }

  return response.json() as T;
};

export {
  cx,
  isValidAddress,
  logger,
  asOptionalField,
  isUndefined,
  isNull,
  isNumeric,
  getTokenSVG,
  getErrorMessage,
  isNativeToken,
  definedAssert,
  fetchWithGuard,
};
