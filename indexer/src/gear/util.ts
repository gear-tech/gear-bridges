import { Event } from './processor';
import { MessageQueuedEvent, UserMessageSentEvent } from './types';

export function isMessageQueued(event: Event): event is MessageQueuedEvent {
  return event.name === 'Gear.MessageQueued';
}

export function isUserMessageSent(event: Event): event is UserMessageSentEvent {
  return event.name === 'Gear.UserMessageSent';
}

export function isProgramChanged(event: Event): boolean {
  return event.name === 'Gear.ProgramChanged';
}

export const enum ProgramName {
  VftManager = 'vft_manager',
  HistoricalProxy = 'historical_proxy',
  BridgingPayment = 'bridging_payment',
}

export const enum VftManagerServices {
  VftManager = 'VftManager',
}

export const enum VftManagerMethods {
  BridgingRequested = 'BridgingRequested',
  TokenMappingAdded = 'TokenMappingAdded',
  TokenMappingRemoved = 'TokenMappingRemoved',
  HistoricalProxyChanged = 'HistoricalProxyChanged', // TODO: check when pr is ready
}

export const enum HistoricalProxyServices {
  HistoricalProxy = 'HistoricalProxy',
}

export const enum HistoricalProxyMethods {
  Relayed = 'Relayed',
}

export const enum BridgingPaymentServices {
  BridgingPayment = 'BridgingPayment',
}

export const enum BridgingPaymentMethods {
  BridgingPaid = 'BridgingPaid',
}
