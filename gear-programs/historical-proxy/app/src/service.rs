use ptr::addr_of_mut;
use sails_rs::{gstd, prelude::*};


pub struct HistoricalProxyService(());

#[sails_rs::service]
impl HistoricalProxyService {
    pub fn new() -> Self {
        Self(())
    }

}
