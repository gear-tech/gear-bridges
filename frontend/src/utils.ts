import { decodeAddress, encodeAddress, ExtrinsicFailedData, HexString } from '@gear-js/api';
import { BaseError } from 'wagmi';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import TokenPlaceholderSVG from '@/assets/token-placeholder.svg?react';

import { TOKEN_SVG } from './consts';

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

const getTokenSVG = (address: HexString) => TOKEN_SVG[address] || TokenPlaceholderSVG;

// string is only for cancelled sign and send popup error during useSendProgramTransaction
// reevaluate after @gear-js/react-hooks update
const getErrorMessage = (error: Error | WriteContractErrorType | ExtrinsicFailedData | string) => {
  if (typeof error === 'object' && 'docs' in error) {
    return error.docs || error.method || error.name;
  }

  return typeof error === 'string' ? error : (error as BaseError).shortMessage || error.message;
};

export { cx, isValidAddress, logger, asOptionalField, isUndefined, isNumeric, getTokenSVG, getErrorMessage };
