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

    pub fn alloc_string(&mut self, bytes: &[u8]) -> Option<*mut u8> {
        let total = core::mem::size_of::<StringHeader>() + bytes.len() + 1;
        let raw = self.mem.alloc(total, core::mem::align_of::<usize>())?;
        unsafe {
            let header = raw as *mut StringHeader;
            (*header).tag = HEAP_TAG_STRING;
            (*header).len = bytes.len() as u32;
            let data = raw.add(core::mem::size_of::<StringHeader>());
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            *data.add(bytes.len()) = 0;
            Some(raw)
        }
    }

    pub fn string_bytes(&self, val: crate::value::Value) -> Option<&[u8]> {
        if !val.is_ptr() {
            return None;
        }
        let ptr = val.as_ptr() as *const StringHeader;
        unsafe {
            if (*ptr).tag != HEAP_TAG_STRING {
                return None;
            }
            let len = (*ptr).len as usize;
            let data = (ptr as *const u8).add(core::mem::size_of::<StringHeader>());
            Some(core::slice::from_raw_parts(data, len))
        }
    }
}

/// Placeholder for the custom allocator and GC state.
struct MemoryRegion {
    start: *mut u8,
    size: usize,
    offset: usize,
}

impl MemoryRegion {
    fn new(buf: &mut [u8]) -> Self {
        Self {
            start: buf.as_mut_ptr(),
            size: buf.len(),
            offset: 0,
        }
    }

    fn alloc(&mut self, size: usize, align: usize) -> Option<*mut u8> {
        let aligned = align_up(self.offset, align);
        let new_offset = aligned.checked_add(size)?;
        if new_offset > self.size {
            return None;
        }
        let ptr = unsafe { self.start.add(aligned) };
        self.offset = new_offset;
        Some(ptr)
    }
}

#[allow(dead_code)]
fn _value_size_check(_v: Value) {
    // Ensures the type is used while we bootstrap the runtime.
}

const HEAP_TAG_STRING: u32 = 1;

#[repr(C)]
struct StringHeader {
    tag: u32,
    len: u32,
}

fn align_up(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }
    (value + align - 1) & !(align - 1)
}
