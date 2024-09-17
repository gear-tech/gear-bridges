use sails_rs::{
    gstd::msg,
    prelude::{collections::HashSet, *},
};

#[derive(Debug)]
pub(crate) struct AdminConfig {
    pub admins: HashSet<ActorId>,
}

static_storage!(AdminConfig);

pub(crate) fn ensure_is_admin() {
    if !storage().admins.contains(&msg::source()) {
        panic!("Not admin")
    };
}

pub(crate) struct AdminService(());

#[sails_rs::service]
impl AdminService {
    pub fn new() -> Self {
        Self(())
    }

    pub fn admins(&self) -> Vec<ActorId> {
        storage().admins.clone().into_iter().collect()
    }

    pub fn grant_admin_role(&mut self, to: ActorId) {
        ensure_is_admin();
        storage_mut().admins.insert(to);
    }

    pub fn revoke_admin_role(&mut self, from: ActorId) {
        ensure_is_admin();
        storage_mut().admins.remove(&from);
    }
}
