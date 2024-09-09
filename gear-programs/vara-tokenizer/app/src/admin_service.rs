use sails_rs::{
    gstd::msg,
    prelude::{collections::HashSet, *},
};

static mut STORAGE: Option<AdminConfig> = None;

#[derive(Debug)]
pub(crate) struct AdminConfig {
    pub admins: HashSet<ActorId>,
}

pub(crate) fn init(admin: ActorId) {
    unsafe {
        STORAGE = Some(AdminConfig {
            admins: [admin].into(),
        });
    };
}

pub(crate) fn storage_mut() -> &'static mut AdminConfig {
    unsafe { STORAGE.as_mut().expect("program is not initialized") }
}

pub(crate) fn storage() -> &'static AdminConfig {
    unsafe { STORAGE.as_ref().expect("program is not initialized") }
}

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
        if storage().admins.len() == 1 {
            panic!("Can't revoke last admin role")
        }
        storage_mut().admins.remove(&from);
    }
}
