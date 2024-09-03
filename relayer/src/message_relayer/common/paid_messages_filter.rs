use std::{
    collections::HashMap,
    sync::mpsc::{channel, Receiver, Sender},
};

use super::{message_paid_listener::PaidMessage, MessageInBlock};

pub struct PaidMessagesFilter {
    pending_messages: HashMap<[u8; 32], MessageInBlock>,
    pending_nonces: Vec<[u8; 32]>,
}

impl PaidMessagesFilter {
    pub fn new() -> Self {
        Self {
            pending_messages: HashMap::default(),
            pending_nonces: vec![],
        }
    }

    pub fn run(
        mut self,
        messages: Receiver<MessageInBlock>,
        paid_messages: Receiver<PaidMessage>,
    ) -> Receiver<MessageInBlock> {
        let (sender, receiver) = channel();

        tokio::spawn(async move {
            loop {
                let res = self.run_inner(&sender, &messages, &paid_messages);
                if let Err(err) = res {
                    log::error!("Paid messages filter failed: {}", err);
                }
            }
        });

        receiver
    }

    fn run_inner(
        &mut self,
        sender: &Sender<MessageInBlock>,
        messages: &Receiver<MessageInBlock>,
        paid_messages: &Receiver<PaidMessage>,
    ) -> anyhow::Result<()> {
        loop {
            for message in messages.try_iter() {
                self.pending_messages
                    .insert(message.message.nonce_le, message)
                    .expect("Received 2 messages with the same nonce");
            }

            for PaidMessage { nonce } in paid_messages.try_iter() {
                self.pending_nonces.push(nonce);
            }

            for i in (0..self.pending_nonces.len()).rev() {
                if let Some(message) = self.pending_messages.remove(&self.pending_nonces[i]) {
                    sender.send(message)?;
                    self.pending_nonces.remove(i);
                }
            }
        }
    }
}
