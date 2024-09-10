// macros
macro_rules! static_storage {
    ($type:ty) => {
        static mut STORAGE: Option<$type> = None;

        pub(crate) fn init(init_value: $type) {
            unsafe {
                STORAGE = Some(init_value);
            };
        }

        pub(crate) fn storage_mut() -> &'static mut $type {
            unsafe { STORAGE.as_mut().expect("program is not initialized") }
        }

        pub(crate) fn storage() -> &'static $type {
            unsafe { STORAGE.as_ref().expect("program is not initialized") }
        }
    };
}
