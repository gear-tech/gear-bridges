use super::*;

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    Added {
        eth_address: H160,
        fungible_token: ActorId,
    },
    Removed {
        eth_address: H160,
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

    pub fn tokens(&self) -> Vec<(H160, ActorId)> {
        self.state.borrow().map.clone()
    }

    pub fn add_fungible_token(&mut self, eth_address: H160, address: ActorId) -> Option<()> {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        match state
            .map
            .binary_search_by(|(eth_address_old, _address)| eth_address_old.cmp(&eth_address))
        {
            Ok(_) => None,
            Err(i) => {
                state.map.insert(i, (eth_address, address));
                self.notify_on(Event::Added {
                    eth_address,
                    fungible_token: address,
                })
                .expect("Notify on add");

                Some(())
            }
        }
    }

    pub fn remove_fungible_token(&mut self, eth_address: H160) -> Option<()> {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        match state
            .map
            .binary_search_by(|(eth_address_old, _address)| eth_address_old.cmp(&eth_address))
        {
            Ok(i) => {
                state.map.remove(i);
                self.notify_on(Event::Removed { eth_address })
                    .expect("Notify on remove");

                Some(())
            }
            Err(_) => None,
        }
    }
}
