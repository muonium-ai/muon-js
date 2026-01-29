use crate::types::JSObjectClassEnum;
use crate::value::Value;

/// Core runtime state. This will evolve to match MQuickJS JSContext.
pub struct Context {
    mem: MemoryRegion,
    gcref_head: *mut crate::types::JSGCRef,
    opaque: *mut core::ffi::c_void,
    interrupt_handler: Option<crate::types::JSInterruptHandler>,
    log_func: Option<crate::types::JSWriteFunc>,
    random_seed: u64,
    atoms: AtomTable,
    global_object: Value,
}

impl Context {
    pub fn new(mem: &mut [u8]) -> Self {
        let mut ctx = Self {
            mem: MemoryRegion::new(mem),
            gcref_head: core::ptr::null_mut(),
            opaque: core::ptr::null_mut(),
            interrupt_handler: None,
            log_func: None,
            random_seed: 0,
            atoms: AtomTable::new(),
            global_object: Value::UNDEFINED,
        };
        if let Some(obj) = ctx.new_object(JSObjectClassEnum::Object as u32) {
            ctx.global_object = obj;
        }
        ctx
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

    pub fn global_object(&self) -> Value {
        self.global_object
    }

    pub fn new_object(&mut self, class_id: u32) -> Option<Value> {
        let obj = self.alloc_object(class_id)?;
        Some(Value::from_ptr(obj as *mut u8))
    }

    pub fn new_array(&mut self, initial_len: usize) -> Option<Value> {
        let obj = self.alloc_array(initial_len)?;
        Some(Value::from_ptr(obj as *mut u8))
    }

    pub fn object_class_id(&self, val: Value) -> Option<u32> {
        let obj = self.object_ptr(val)?;
        unsafe { Some((*obj).class_id) }
    }

    pub fn set_object_opaque(&mut self, val: Value, opaque: *mut core::ffi::c_void) -> bool {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
            None => return false,
        };
        unsafe {
            (*obj).opaque = opaque;
        }
        true
    }

    pub fn get_object_opaque(&self, val: Value) -> *mut core::ffi::c_void {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
            None => return core::ptr::null_mut(),
        };
        unsafe { (*obj).opaque }
    }

    pub fn get_property_str(&mut self, val: Value, name: &[u8]) -> Option<Value> {
        let obj = self.object_ptr(val)?;
        unsafe {
            if (*obj).tag == HEAP_TAG_ARRAY && name == b"length" {
                return Some(Value::from_int32((*obj).array_len as i32));
            }
        }
        let atom = self.intern_string(name)?;
        unsafe { self.find_prop_value(obj, PROP_KEY_ATOM, atom) }
    }

    pub fn get_property_index(&mut self, val: Value, idx: u32) -> Option<Value> {
        let obj = self.object_ptr(val)?;
        unsafe {
            if (*obj).tag == HEAP_TAG_ARRAY {
                return Some(self.array_get(obj, idx));
            }
        }
        unsafe { self.find_prop_value(obj, PROP_KEY_INDEX, idx) }
    }

    pub fn set_property_str(&mut self, val: Value, name: &[u8], value: Value) -> bool {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
            None => return false,
        };
        unsafe {
            if (*obj).tag == HEAP_TAG_ARRAY && name == b"length" {
                return self.array_set_length(obj, value).is_ok();
            }
        }
        let atom = match self.intern_string(name) {
            Some(atom) => atom,
            None => return false,
        };
        unsafe { self.set_prop_value(obj, PROP_KEY_ATOM, atom, value) }
    }

    pub fn set_property_index(&mut self, val: Value, idx: u32, value: Value) -> Result<(), ()> {
        let obj = self.object_ptr(val).ok_or(())?;
        unsafe {
            if (*obj).tag == HEAP_TAG_ARRAY {
                return self.array_set(obj, idx, value);
            }
        }
        if unsafe { self.set_prop_value(obj, PROP_KEY_INDEX, idx, value) } {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn intern_string(&mut self, bytes: &[u8]) -> Option<u32> {
        if let Some(id) = self.atoms.find(bytes) {
            return Some(id);
        }
        let header = self.alloc_string(bytes)?;
        let id = self.atoms.push(AtomEntry { bytes: header })?;
        Some(id)
    }

    pub fn atom_bytes(&self, id: u32) -> Option<&[u8]> {
        let entry = self.atoms.get(id)?;
        let header = entry.bytes as *const StringHeader;
        unsafe {
            if (*header).tag != HEAP_TAG_STRING {
                return None;
            }
            let len = (*header).len as usize;
            let data = (header as *const u8).add(core::mem::size_of::<StringHeader>());
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
const HEAP_TAG_OBJECT: u32 = 2;
const HEAP_TAG_ARRAY: u32 = 3;

const PROP_KEY_ATOM: u32 = 0;
const PROP_KEY_INDEX: u32 = 1;

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

struct AtomTable {
    entries: Vec<AtomEntry>,
}

impl AtomTable {
    fn new() -> Self {
        let mut table = Self { entries: Vec::new() };
        table.entries.push(AtomEntry { bytes: core::ptr::null_mut() });
        table
    }

    fn find(&self, bytes: &[u8]) -> Option<u32> {
        for (idx, entry) in self.entries.iter().enumerate() {
            if entry.bytes_equal(bytes) {
                return Some(idx as u32);
            }
        }
        None
    }

    fn push(&mut self, entry: AtomEntry) -> Option<u32> {
        let id = self.entries.len() as u32;
        self.entries.push(entry);
        Some(id)
    }

    fn get(&self, id: u32) -> Option<&AtomEntry> {
        self.entries.get(id as usize)
    }
}

struct AtomEntry {
    bytes: *mut u8,
}

impl AtomEntry {
    fn bytes_equal(&self, bytes: &[u8]) -> bool {
        if self.bytes.is_null() {
            return false;
        }
        let header = self.bytes as *const StringHeader;
        unsafe {
            if (*header).tag != HEAP_TAG_STRING {
                return false;
            }
            let len = (*header).len as usize;
            if len != bytes.len() {
                return false;
            }
            let data = (header as *const u8).add(core::mem::size_of::<StringHeader>());
            let stored = core::slice::from_raw_parts(data, len);
            stored == bytes
        }
    }
}

#[repr(C)]
struct HeapObject {
    tag: u32,
    class_id: u32,
    prop_head: *mut Property,
    prop_count: u32,
    array_len: u32,
    array_cap: u32,
    elements: *mut Value,
    opaque: *mut core::ffi::c_void,
}

#[repr(C)]
struct Property {
    next: *mut Property,
    key_kind: u32,
    key: u32,
    value: Value,
}

impl Context {
    fn alloc_object(&mut self, class_id: u32) -> Option<*mut HeapObject> {
        let raw = self.mem.alloc(core::mem::size_of::<HeapObject>(), core::mem::align_of::<usize>())?;
        unsafe {
            let obj = raw as *mut HeapObject;
            let tag = if class_id == JSObjectClassEnum::Array as u32 {
                HEAP_TAG_ARRAY
            } else {
                HEAP_TAG_OBJECT
            };
            (*obj).tag = tag;
            (*obj).class_id = class_id;
            (*obj).prop_head = core::ptr::null_mut();
            (*obj).prop_count = 0;
            (*obj).array_len = 0;
            (*obj).array_cap = 0;
            (*obj).elements = core::ptr::null_mut();
            (*obj).opaque = core::ptr::null_mut();
            Some(obj)
        }
    }

    fn alloc_array(&mut self, initial_len: usize) -> Option<*mut HeapObject> {
        let obj = self.alloc_object(JSObjectClassEnum::Array as u32)?;
        let cap = if initial_len == 0 { 0 } else { initial_len };
        let elements = if cap == 0 {
            core::ptr::null_mut()
        } else {
            let size = cap.checked_mul(core::mem::size_of::<Value>())?;
            let ptr = self.mem.alloc(size, core::mem::align_of::<Value>())?;
            unsafe {
                let vals = core::slice::from_raw_parts_mut(ptr as *mut Value, cap);
                for v in vals.iter_mut() {
                    *v = Value::UNDEFINED;
                }
            }
            ptr as *mut Value
        };
        unsafe {
            (*obj).array_len = initial_len as u32;
            (*obj).array_cap = cap as u32;
            (*obj).elements = elements;
        }
        Some(obj)
    }

    fn object_ptr(&self, val: Value) -> Option<*mut HeapObject> {
        if !val.is_ptr() {
            return None;
        }
        let ptr = val.as_ptr();
        if ptr.is_null() {
            return None;
        }
        unsafe {
            let tag = *(ptr as *const u32);
            if tag == HEAP_TAG_OBJECT || tag == HEAP_TAG_ARRAY {
                Some(ptr as *mut HeapObject)
            } else {
                None
            }
        }
    }

    unsafe fn find_prop_value(&self, obj: *mut HeapObject, kind: u32, key: u32) -> Option<Value> {
        let mut cur = (*obj).prop_head;
        while !cur.is_null() {
            if (*cur).key_kind == kind && (*cur).key == key {
                return Some((*cur).value);
            }
            cur = (*cur).next;
        }
        Some(Value::UNDEFINED)
    }

    unsafe fn set_prop_value(&mut self, obj: *mut HeapObject, kind: u32, key: u32, value: Value) -> bool {
        let mut cur = (*obj).prop_head;
        while !cur.is_null() {
            if (*cur).key_kind == kind && (*cur).key == key {
                (*cur).value = value;
                return true;
            }
            cur = (*cur).next;
        }
        let prop = self.alloc_property(kind, key, value);
        if prop.is_null() {
            return false;
        }
        (*prop).next = (*obj).prop_head;
        (*obj).prop_head = prop;
        (*obj).prop_count = (*obj).prop_count.saturating_add(1);
        true
    }

    fn alloc_property(&mut self, kind: u32, key: u32, value: Value) -> *mut Property {
        let raw = match self.mem.alloc(core::mem::size_of::<Property>(), core::mem::align_of::<usize>()) {
            Some(ptr) => ptr,
            None => return core::ptr::null_mut(),
        };
        unsafe {
            let prop = raw as *mut Property;
            (*prop).next = core::ptr::null_mut();
            (*prop).key_kind = kind;
            (*prop).key = key;
            (*prop).value = value;
            prop
        }
    }

    unsafe fn array_get(&self, obj: *mut HeapObject, idx: u32) -> Value {
        if idx >= (*obj).array_len {
            return Value::UNDEFINED;
        }
        if (*obj).elements.is_null() {
            return Value::UNDEFINED;
        }
        *(*obj).elements.add(idx as usize)
    }

    unsafe fn array_set(&mut self, obj: *mut HeapObject, idx: u32, value: Value) -> Result<(), ()> {
        let len = (*obj).array_len as usize;
        if idx as usize > len {
            return Err(());
        }
        if idx as usize == len {
            let new_len = len + 1;
            if new_len > (*obj).array_cap as usize {
                self.array_grow(obj, new_len)?;
            }
            if (*obj).elements.is_null() {
                return Err(());
            }
            *(*obj).elements.add(len) = value;
            (*obj).array_len = new_len as u32;
            return Ok(());
        }
        if (*obj).elements.is_null() {
            return Err(());
        }
        *(*obj).elements.add(idx as usize) = value;
        Ok(())
    }

    unsafe fn array_set_length(&mut self, obj: *mut HeapObject, value: Value) -> Result<(), ()> {
        let new_len = match value.int32() {
            Some(v) if v >= 0 => v as usize,
            _ => return Err(()),
        };
        let current = (*obj).array_len as usize;
        if new_len > current {
            return Err(());
        }
        (*obj).array_len = new_len as u32;
        Ok(())
    }

    unsafe fn array_grow(&mut self, obj: *mut HeapObject, needed: usize) -> Result<(), ()> {
        let current = (*obj).array_cap as usize;
        let mut new_cap = if current == 0 { 4 } else { current * 2 };
        if new_cap < needed {
            new_cap = needed;
        }
        let size = new_cap.checked_mul(core::mem::size_of::<Value>()).ok_or(())?;
        let raw = self.mem.alloc(size, core::mem::align_of::<Value>()).ok_or(())?;
        let new_elems = raw as *mut Value;
        for i in 0..new_cap {
            *new_elems.add(i) = Value::UNDEFINED;
        }
        if !(*obj).elements.is_null() {
            let old_len = (*obj).array_len as usize;
            for i in 0..old_len {
                *new_elems.add(i) = *(*obj).elements.add(i);
            }
        }
        (*obj).elements = new_elems;
        (*obj).array_cap = new_cap as u32;
        Ok(())
    }
}
