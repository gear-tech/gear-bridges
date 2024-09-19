use super::*;

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    Added {
        eth_address: H160,
        eth_token: H160,
        vara_token: ActorId,
    },
    Removed {
        eth_address: H160,
        eth_token: H160,
    },
}

pub struct FTManage<'a, ExecContext> {
    state: &'a RefCell<State>,
    exec_context: ExecContext,
}

#[sails_rs::service(events = Event)]
impl<'a, T> FTManage<'a, T>
where
    T: ExecContext,
{
    pub fn new(state: &'a RefCell<State>, exec_context: T) -> Self {
        Self {
            state,
            exec_context,
        }
    }

    pub fn tokens(&self) -> Vec<(H160, H160, ActorId)> {
        self.state
            .borrow()
            .map
            .iter()
            .map(|((eth_address, eth_token), vara_token)| (*eth_address, *eth_token, *vara_token))
            .collect()
    }

    pub fn add_fungible_token(
        &mut self,
        eth_address: H160,
        eth_token: H160,
        vara_token: ActorId,
    ) -> Option<()> {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        if state.map.contains_key(&(eth_address, eth_token)) {
            return None;
        }

        state.map.insert((eth_address, eth_token), vara_token);
        self.notify_on(Event::Added {
            eth_address,
            eth_token,
            vara_token,
        })
        .expect("Notify on add");

        Some(())
    }

    pub fn remove_fungible_token(&mut self, eth_address: H160, eth_token: H160) -> Option<()> {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        state.map.remove(&(eth_address, eth_token)).map(|_| {
            self.notify_on(Event::Removed {
                eth_address,
                eth_token,
            })
            .expect("Notify on remove");
        })
    }
}
