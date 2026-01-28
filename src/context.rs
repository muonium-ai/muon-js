use crate::value::Value;

/// Core runtime state. This will evolve to match MQuickJS JSContext.
pub struct Context {
    mem: MemoryRegion,
    gcref_head: *mut crate::types::JSGCRef,
    opaque: *mut core::ffi::c_void,
    interrupt_handler: Option<crate::types::JSInterruptHandler>,
    log_func: Option<crate::types::JSWriteFunc>,
    random_seed: u64,
}

impl Context {
    pub fn new(mem: &mut [u8]) -> Self {
        Self {
            mem: MemoryRegion::new(mem),
            gcref_head: core::ptr::null_mut(),
            opaque: core::ptr::null_mut(),
            interrupt_handler: None,
            log_func: None,
            random_seed: 0,
        }
    }

    pub fn gcref_head(&mut self) -> *mut crate::types::JSGCRef {
        self.gcref_head
    }

    pub fn set_gcref_head(&mut self, head: *mut crate::types::JSGCRef) {
        self.gcref_head = head;
    }

    pub fn set_opaque(&mut self, opaque: *mut core::ffi::c_void) {
        self.opaque = opaque;
    }

    pub fn opaque(&self) -> *mut core::ffi::c_void {
        self.opaque
    }

    pub fn set_interrupt_handler(&mut self, handler: Option<crate::types::JSInterruptHandler>) {
        self.interrupt_handler = handler;
    }

    pub fn interrupt_handler(&self) -> Option<crate::types::JSInterruptHandler> {
        self.interrupt_handler
    }

    pub fn set_log_func(&mut self, log_func: Option<crate::types::JSWriteFunc>) {
        self.log_func = log_func;
    }

    pub fn log_func(&self) -> Option<crate::types::JSWriteFunc> {
        self.log_func
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random_seed = seed;
    }

    pub fn random_seed(&self) -> u64 {
        self.random_seed
    }
}

/// Placeholder for the custom allocator and GC state.
struct MemoryRegion {
    start: *mut u8,
    size: usize,
}

impl MemoryRegion {
    fn new(buf: &mut [u8]) -> Self {
        Self {
            start: buf.as_mut_ptr(),
            size: buf.len(),
        }
    }
}

#[allow(dead_code)]
fn _value_size_check(_v: Value) {
    // Ensures the type is used while we bootstrap the runtime.
}
