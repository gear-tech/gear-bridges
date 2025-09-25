import { Event } from '../processor.js';

export type EthBridgeMessageQueuedEvent = Omit<Event, 'args'> & { args: EthBridgeMessageQueuedArgs };

export interface EthBridgeMessageQueuedArgs {
  readonly message: {
    readonly nonce: string;
    readonly source: string;
    readonly destination: string;
    readonly payload: string;
  };
  readonly hash: string;
}

export type QueueMerkleRootChangedEvent = Omit<Event, 'args'> & { args: QueueMerkleRootChangedArgs };

export interface QueueMerkleRootChangedArgs {
  readonly queueId: number;
  readonly root: string;
}

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
    readonly details: UserMessageSentDetails;
  };
}

export interface UserMessageSentDetails {
  readonly to: string;
  readonly code: {
    readonly __kind: 'Success' | 'Error';
  };
}

export type ProgramChangedEvent = Omit<Event, 'args'> & { args: ProgramChangedArgs };

export interface ProgramChangedArgs {
  readonly id: `0x${string}`;
  readonly change: {
    readonly __kind: 'Active' | 'Inactive';
  };
}
