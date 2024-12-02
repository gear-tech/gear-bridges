import { Event } from '../processor';

export type MessageQueuedEvent = Omit<Event, 'args'> & { args: MessageQueuedArgs };

export interface MessageQueuedArgs {
  readonly id: string;
  readonly source: string;
  readonly destination: string;
  readonly entry: 'Init' | 'Handle' | 'Reply';
}

export type UserMessageSentEvent = Omit<Event, 'args'> & { args: UserMessageSentArgs };

export interface UserMessageSentArgs {
  readonly message: {
    readonly id: string;
    readonly source: string;
    readonly destination: string;
    readonly payload: `0x${string}`;
    readonly value: string;
    readonly details: UserMessageSentArgs;
  };
}

export interface UserMessageSentDetails {
  readonly to: string;
  readonly code: {
    readonly __kind: 'Success' | 'Error';
  };
}
