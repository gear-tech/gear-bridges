import { Status } from '../types';
import { getLastDaysISOTimestamp } from '../utils';

const FIELD_NAME = {
  TIMESTAMP: 'timestamp',
  STATUS: 'status',
  ASSET: 'asset',
  SEARCH: 'search',
  OWNER: 'owner',
} as const;

const TIMESTAMP_OPTIONS = [
  { label: 'All Time', value: '' },
  { label: 'Last 7 days', value: getLastDaysISOTimestamp(7) },
  { label: 'Last 30 days', value: getLastDaysISOTimestamp(30) },
  { label: 'Last 90 days', value: getLastDaysISOTimestamp(90) },
  { label: 'Last 180 days', value: getLastDaysISOTimestamp(180) },
  { label: 'Last 365 days', value: getLastDaysISOTimestamp(365) },
] as const;

const STATUS_OPTIONS = [
  { label: 'All Statuses', value: '' },
  { label: 'Completed', value: Status.Completed },
  { label: 'Failed', value: Status.Failed },
  { label: 'Awaiting Payment', value: Status.AwaitingPayment },
  { label: 'Bridging', value: Status.Bridging },
];

const DEFAULT_VALUES = {
  [FIELD_NAME.TIMESTAMP]: TIMESTAMP_OPTIONS[0].value as string,
  [FIELD_NAME.STATUS]: STATUS_OPTIONS[0].value as Status | '',
  [FIELD_NAME.ASSET]: '',
  [FIELD_NAME.SEARCH]: '',
  [FIELD_NAME.OWNER]: false,
};

export { FIELD_NAME, DEFAULT_VALUES, TIMESTAMP_OPTIONS, STATUS_OPTIONS };
