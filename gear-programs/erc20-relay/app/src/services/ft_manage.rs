use super::*;

#[derive(Encode, TypeInfo)]
#[codec(crate = sails_rs::scale_codec)]
#[scale_info(crate = sails_rs::scale_info)]
enum Event {
    Added { erc20_treasury: H160 },
    Removed { erc20_treasury: H160 },
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

    pub fn addresses(&self) -> Vec<H160> {
        self.state.borrow().addresses.iter().copied().collect()
    }

    pub fn add_erc_treasury(&mut self, erc20_treasury: H160) -> Option<()> {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        if state.addresses.contains(&erc20_treasury) {
            return None;
        }

        state.addresses.insert(erc20_treasury);
        self.notify_on(Event::Added { erc20_treasury })
            .expect("Notify on add");

        Some(())
    }

    pub fn remove_erc_treasury(&mut self, erc20_treasury: H160) -> Option<()> {
        let source = self.exec_context.actor_id();
        let mut state = self.state.borrow_mut();
        if source != state.admin {
            panic!("Not admin");
        }

        if state.addresses.remove(&erc20_treasury) {
            self.notify_on(Event::Removed { erc20_treasury })
                .expect("Notify on remove");

            return Some(());
        }

        None
    }
}
