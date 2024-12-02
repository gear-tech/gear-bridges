import { Event } from './processor';
import { MessageQueuedEvent, UserMessageSentEvent } from './types';

export function isMessageQueued(event: Event): event is MessageQueuedEvent {
  return event.name === 'Gear.MessageQueued';
}

export function isUserMessageSent(event: Event): event is UserMessageSentEvent {
  return event.name === 'Gear.UserMessageSent';
}
