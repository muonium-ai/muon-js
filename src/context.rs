use crate::types::{JSObjectClassEnum, JSWord, JSSTDLibraryDef};
use crate::value::Value;

use std::collections::HashMap;

const PROTO_SEARCH_LIMIT: usize = 64;
const MAX_ROM_ATOM_TABLES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopControl {
    None,
    Break,
    Continue,
    Return,
    BreakLabel(String),
    ContinueLabel(String),
}

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
    call_stack: Vec<Value>,
    stack_limit: usize,
    last_exception: Value,
    c_function_table: *const crate::types::JSCFunctionDef,
    c_function_table_len: usize,
    object_proto: Value,
    array_proto: Value,
    array_push_fn: Value,
    array_pop_fn: Value,
    loop_control: LoopControl,
    return_value: Value,
    env_stack: Vec<Value>,
    gc_objects: Vec<*mut u8>,
    gc_marks: Vec<*mut u8>,
    rom_atom_tables: Vec<Value>,
    stdlib_table: *const JSWord,
    stdlib_table_len: u32,
    stdlib_table_align: u32,
    stdlib_sorted_atoms_offset: u32,
    stdlib_global_object_offset: u32,
    stdlib_class_count: u32,
    current_filename: String,
    current_source: String,
    current_line_starts: Vec<usize>,
    current_error_offset: Option<usize>,
    current_stmt_offset: usize,
    /// Cache of parsed function bodies: body Value raw bits → pre-split statements.
    body_cache: HashMap<u64, Vec<String>>,
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
            call_stack: Vec::new(),
            stack_limit: 1024,
            last_exception: Value::UNDEFINED,
            c_function_table: core::ptr::null(),
            c_function_table_len: 0,
            return_value: Value::UNDEFINED,
            object_proto: Value::UNDEFINED,
            array_proto: Value::UNDEFINED,
            array_push_fn: Value::UNDEFINED,
            array_pop_fn: Value::UNDEFINED,
            loop_control: LoopControl::None,
            env_stack: Vec::new(),
            gc_objects: Vec::new(),
            gc_marks: Vec::new(),
            rom_atom_tables: Vec::new(),
            stdlib_table: core::ptr::null(),
            stdlib_table_len: 0,
            stdlib_table_align: 0,
            stdlib_sorted_atoms_offset: 0,
            stdlib_global_object_offset: 0,
            stdlib_class_count: 0,
            current_filename: String::new(),
            current_source: String::new(),
            current_line_starts: Vec::new(),
            current_error_offset: None,
            current_stmt_offset: 0,
            body_cache: HashMap::new(),
        };
        if let Some(obj) = ctx.new_object(JSObjectClassEnum::Object as u32) {
            ctx.global_object = obj;
        }
        ctx
    }

    pub fn push_arg(&mut self, val: Value) {
        self.call_stack.push(val);
    }

    pub fn stack_check(&self, len: u32) -> i32 {
        if self.call_stack.len() + len as usize <= self.stack_limit {
            0
        } else {
            1
        }
    }

    pub fn call(&mut self, call_flags: i32) -> Value {
        let argc = (call_flags & 0xffff) as usize;
        let need = argc + 2;
        if self.call_stack.len() < need {
            return Value::EXCEPTION;
        }
        for _ in 0..need {
            let _ = self.call_stack.pop();
        }
        Value::EXCEPTION
    }

    pub fn call_stack_len(&self) -> usize {
        self.call_stack.len()
    }

    pub fn call_stack_get(&self, idx: usize) -> Value {
        self.call_stack[idx]
    }

    pub fn call_stack_truncate(&mut self, len: usize) {
        self.call_stack.truncate(len);
    }

    pub fn set_exception(&mut self, val: Value) {
        self.last_exception = val;
    }

    pub fn get_exception(&self) -> Value {
        self.last_exception
    }

    pub fn set_current_source(&mut self, filename: &str, source: &str) {
        self.current_filename = filename.to_string();
        self.current_source = source.to_string();
        self.current_line_starts.clear();
        self.current_line_starts.push(0);
        for (i, b) in source.as_bytes().iter().enumerate() {
            if *b == b'\n' {
                if i + 1 < source.len() {
                    self.current_line_starts.push(i + 1);
                }
            }
        }
        self.current_error_offset = None;
        self.current_stmt_offset = 0;
    }

    pub fn current_source(&self) -> &str {
        &self.current_source
    }

    pub fn current_filename(&self) -> &str {
        if self.current_filename.is_empty() {
            "<eval>"
        } else {
            &self.current_filename
        }
    }

    pub fn set_error_offset(&mut self, offset: usize) {
        self.current_error_offset = Some(offset);
    }

    pub fn clear_error_offset(&mut self) {
        self.current_error_offset = None;
    }

    pub fn current_error_offset(&self) -> Option<usize> {
        self.current_error_offset
    }

    pub fn set_current_stmt_offset(&mut self, offset: usize) {
        self.current_stmt_offset = offset;
    }

    pub fn current_stmt_offset(&self) -> usize {
        self.current_stmt_offset
    }

    /// Look up cached parsed statements for a function body Value.
    pub fn get_body_cache(&self, body_val_bits: u64) -> Option<&Vec<String>> {
        self.body_cache.get(&body_val_bits)
    }

    /// Store cached parsed statements for a function body Value.
    pub fn set_body_cache(&mut self, body_val_bits: u64, stmts: Vec<String>) {
        self.body_cache.insert(body_val_bits, stmts);
    }

    pub fn compute_line_col(&self, offset: usize) -> (usize, usize) {
        let end = offset.min(self.current_source.len());
        if self.current_line_starts.is_empty() {
            return (1, end + 1);
        }
        let mut lo = 0usize;
        let mut hi = self.current_line_starts.len();
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.current_line_starts[mid] <= end {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let line_idx = lo.saturating_sub(1);
        let line_start = self.current_line_starts[line_idx];
        let line = line_idx + 1;
        let col = end.saturating_sub(line_start) + 1;
        (line, col)
    }

    pub fn format_stack(&self) -> Option<String> {
        let offset = self.current_error_offset?;
        let (line, col) = self.compute_line_col(offset);
        let filename = self.current_filename();
        Some(format!(
            "at {}:{}:{}\nat {}:{}:{}",
            filename, line, col, filename, line, col
        ))
    }

    pub fn set_loop_control(&mut self, ctrl: LoopControl) {
        self.loop_control = ctrl;
    }

    pub fn get_loop_control(&self) -> &LoopControl {
        &self.loop_control
    }

    /// Check if a labeled break matches the given label
    pub fn is_break_label(&self, label: &str) -> bool {
        matches!(&self.loop_control, LoopControl::BreakLabel(l) if l == label)
    }

    /// Check if a labeled continue matches the given label
    pub fn is_continue_label(&self, label: &str) -> bool {
        matches!(&self.loop_control, LoopControl::ContinueLabel(l) if l == label)
    }

    pub fn set_return_value(&mut self, val: Value) {
        self.return_value = val;
    }

    pub fn get_return_value(&self) -> Value {
        self.return_value
    }

    pub fn current_env(&self) -> Value {
        match self.env_stack.last() {
            Some(env) => *env,
            None => self.global_object,
        }
    }

    pub fn push_env(&mut self, env: Value) {
        self.env_stack.push(env);
    }

    pub fn pop_env(&mut self) {
        self.env_stack.pop();
    }

    pub fn resolve_binding(&mut self, name: &str) -> Option<(Value, Value)> {
        let mut env = self.current_env();
        let global = self.global_object;
        loop {
            if self.has_property_str(env, name.as_bytes()) {
                let val = self.get_property_str(env, name.as_bytes()).unwrap_or(Value::UNDEFINED);
                return Some((env, val));
            }
            if env == global {
                break;
            }
            let parent = self
                .get_property_str(env, b"__parent__")
                .unwrap_or(Value::UNDEFINED);
            if parent.is_undefined() {
                break;
            }
            env = parent;
        }
        if self.has_property_str(global, name.as_bytes()) {
            let val = self
                .get_property_str(global, name.as_bytes())
                .unwrap_or(Value::UNDEFINED);
            return Some((global, val));
        }
        None
    }

    pub fn resolve_binding_env(&mut self, name: &str) -> Option<Value> {
        self.resolve_binding(name).map(|(env, _)| env)
    }

    pub fn resolve_binding_value(&mut self, name: &str) -> Value {
        self.resolve_binding(name)
            .map(|(_, val)| val)
            .unwrap_or(Value::UNDEFINED)
    }

    pub fn current_var_env(&mut self) -> Value {
        let mut env = self.current_env();
        loop {
            if self.has_property_str(env, b"__var_env__") {
                return env;
            }
            let parent = self
                .get_property_str(env, b"__parent__")
                .unwrap_or(Value::UNDEFINED);
            if parent.is_undefined() {
                break;
            }
            env = parent;
        }
        self.global_object
    }

    pub fn set_c_function_table(&mut self, ptr: *const crate::types::JSCFunctionDef, len: usize) {
        self.c_function_table = ptr;
        self.c_function_table_len = len;
    }

    pub fn set_stdlib_def(&mut self, def: &JSSTDLibraryDef) {
        self.stdlib_table = def.stdlib_table;
        self.stdlib_table_len = def.stdlib_table_len;
        self.stdlib_table_align = def.stdlib_table_align;
        self.stdlib_sorted_atoms_offset = def.sorted_atoms_offset;
        self.stdlib_global_object_offset = def.global_object_offset;
        self.stdlib_class_count = def.class_count;
    }

    pub fn add_rom_atom_table(&mut self, table: Value) -> bool {
        if self.rom_atom_tables.len() >= MAX_ROM_ATOM_TABLES {
            return false;
        }
        self.rom_atom_tables.push(table);
        true
    }

    pub fn set_array_proto_methods(&mut self, push: Value, pop: Value) {
        self.array_push_fn = push;
        self.array_pop_fn = pop;
    }

    pub fn set_object_proto_default(&mut self, proto: Value) {
        self.object_proto = proto;
    }

    pub fn object_proto_default(&self) -> Value {
        self.object_proto
    }

    pub fn array_proto(&self) -> Value {
        self.array_proto
    }

    pub fn set_array_proto(&mut self, proto: Value) {
        self.array_proto = proto;
    }

    pub fn c_function_def(&self, idx: usize) -> Option<&crate::types::JSCFunctionDef> {
        if self.c_function_table.is_null() || idx >= self.c_function_table_len {
            return None;
        }
        unsafe { Some(&*self.c_function_table.add(idx)) }
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

    pub fn write_log(&self, bytes: &[u8]) {
        if let Some(log) = self.log_func {
            log(self.opaque, bytes.as_ptr(), bytes.len());
        }
    }

    pub fn set_random_seed(&mut self, seed: u64) {
        self.random_seed = seed;
    }

    pub fn random_seed(&self) -> u64 {
        self.random_seed
    }

    pub fn memory_usage(&self) -> (usize, usize) {
        self.mem.used()
    }

    pub fn gc_collect(&mut self) {
        self.gc_clear_marks();
        let mut stack = Vec::new();
        self.gc_mark_value(self.global_object, &mut stack);
        let envs = self.env_stack.clone();
        for env in envs {
            self.gc_mark_value(env, &mut stack);
        }
        let call_stack = self.call_stack.clone();
        for val in call_stack {
            self.gc_mark_value(val, &mut stack);
        }
        self.gc_mark_value(self.last_exception, &mut stack);
        self.gc_mark_value(self.return_value, &mut stack);
        self.gc_mark_value(self.object_proto, &mut stack);
        self.gc_mark_value(self.array_proto, &mut stack);
        self.gc_mark_value(self.array_push_fn, &mut stack);
        self.gc_mark_value(self.array_pop_fn, &mut stack);
        self.gc_mark_gcref(&mut stack);
        self.atoms.mark_live(&mut stack);

        while let Some(val) = stack.pop() {
            self.gc_mark_value(val, &mut stack);
        }
    }

    pub fn alloc_string(&mut self, bytes: &[u8]) -> Option<*mut u8> {
        let total = core::mem::size_of::<StringHeader>() + bytes.len() + 1;
        let raw = self.mem.alloc(total, core::mem::align_of::<usize>())?;
        self.gc_objects.push(raw);
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
        if class_id == JSObjectClassEnum::Object as u32 && !self.object_proto.is_undefined() {
            unsafe {
                (*obj).proto = self.object_proto;
            }
        }
        Some(Value::from_ptr(obj as *mut u8))
    }

    pub fn new_array(&mut self, initial_len: usize) -> Option<Value> {
        let obj = self.alloc_array(initial_len)?;
        if !self.array_proto.is_undefined() {
            unsafe {
                (*obj).proto = self.array_proto;
            }
        }
        let val = Value::from_ptr(obj as *mut u8);
        if self.array_proto.is_undefined() {
            if !self.array_push_fn.is_undefined() {
                let _ = self.set_property_str(val, b"push", self.array_push_fn);
            }
            if !self.array_pop_fn.is_undefined() {
                let _ = self.set_property_str(val, b"pop", self.array_pop_fn);
            }
        }
        Some(val)
    }

    pub fn new_c_function(&mut self, func_idx: i32, params: Value) -> Option<Value> {
        let obj = self.alloc_object(JSObjectClassEnum::CFunction as u32)?;
        unsafe {
            (*obj).func_idx = func_idx;
            (*obj).func_params = params;
        }
        Some(Value::from_ptr(obj as *mut u8))
    }

    pub fn c_function_info(&self, val: Value) -> Option<(i32, Value)> {
        let obj = self.object_ptr(val)?;
        unsafe {
            if (*obj).class_id != JSObjectClassEnum::CFunction as u32 {
                return None;
            }
            Some(((*obj).func_idx, (*obj).func_params))
        }
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
        if name == b"length" {
            if let Some(bytes) = self.string_bytes(val) {
                return Some(Value::from_int32(bytes.len() as i32));
            }
        }
        if let Some(idx) = parse_index(name) {
            return self.get_property_index(val, idx);
        }
        let atom = self.intern_string(name)?;
        self.get_property_atom(val, atom, name)
    }

    pub fn has_property_str(&mut self, val: Value, name: &[u8]) -> bool {
        if name == b"length" {
            if self.string_bytes(val).is_some() {
                return true;
            }
        }
        if let Some(idx) = parse_index(name) {
            return self.has_property_index(val, idx);
        }
        let atom = match self.intern_string(name) {
            Some(atom) => atom,
            None => return false,
        };
        let mut cur = val;
        let mut depth = 0;
        while depth < PROTO_SEARCH_LIMIT {
            let obj = match self.object_ptr(cur) {
                Some(obj) => obj,
                None => return false,
            };
            unsafe {
                if (*obj).tag == HEAP_TAG_ARRAY && name == b"length" {
                    return true;
                }
                if self.find_prop_value(obj, PROP_KEY_ATOM, atom).is_some() {
                    return true;
                }
                let proto = (*obj).proto;
                if proto.is_null() || proto.is_undefined() {
                    break;
                }
                cur = proto;
            }
            depth += 1;
        }
        false
    }

    fn has_property_index(&mut self, val: Value, idx: u32) -> bool {
        let mut cur = val;
        let mut depth = 0;
        while depth < PROTO_SEARCH_LIMIT {
            let obj = match self.object_ptr(cur) {
                Some(obj) => obj,
                None => return false,
            };
            unsafe {
                if (*obj).tag == HEAP_TAG_ARRAY {
                    if idx < (*obj).array_len {
                        return true;
                    }
                }
                if self.find_prop_value(obj, PROP_KEY_INDEX, idx).is_some() {
                    return true;
                }
                let proto = (*obj).proto;
                if proto.is_null() || proto.is_undefined() {
                    break;
                }
                cur = proto;
            }
            depth += 1;
        }
        false
    }

    fn get_property_atom(&mut self, val: Value, atom: u32, name: &[u8]) -> Option<Value> {
        let mut cur = val;
        let mut depth = 0;
        while depth < PROTO_SEARCH_LIMIT {
            let obj = self.object_ptr(cur)?;
            unsafe {
                if (*obj).tag == HEAP_TAG_ARRAY && name == b"length" {
                    return Some(Value::from_int32((*obj).array_len as i32));
                }
                if let Some(found) = self.find_prop_value(obj, PROP_KEY_ATOM, atom) {
                    return Some(found);
                }
                let proto = (*obj).proto;
                if proto.is_null() || proto.is_undefined() {
                    break;
                }
                cur = proto;
            }
            depth += 1;
        }
        Some(Value::UNDEFINED)
    }

    pub fn get_property_atom_id(&mut self, val: Value, atom: u32) -> Option<Value> {
        let mut cur = val;
        let mut depth = 0;
        while depth < PROTO_SEARCH_LIMIT {
            let obj = self.object_ptr(cur)?;
            unsafe {
                if let Some(found) = self.find_prop_value(obj, PROP_KEY_ATOM, atom) {
                    return Some(found);
                }
                let proto = (*obj).proto;
                if proto.is_null() || proto.is_undefined() {
                    break;
                }
                cur = proto;
            }
            depth += 1;
        }
        Some(Value::UNDEFINED)
    }

    pub fn get_property_index(&mut self, val: Value, idx: u32) -> Option<Value> {
        let mut cur = val;
        let mut depth = 0;
        while depth < PROTO_SEARCH_LIMIT {
            let obj = self.object_ptr(cur)?;
            unsafe {
                if (*obj).tag == HEAP_TAG_ARRAY {
                    if idx < (*obj).array_len {
                        return Some(self.array_get(obj, idx));
                    }
                }
                if let Some(found) = self.find_prop_value(obj, PROP_KEY_INDEX, idx) {
                    return Some(found);
                }
                let proto = (*obj).proto;
                if proto.is_null() || proto.is_undefined() {
                    break;
                }
                cur = proto;
            }
            depth += 1;
        }
        Some(Value::UNDEFINED)
    }

    pub fn set_property_str(&mut self, val: Value, name: &[u8], value: Value) -> bool {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
            None => return false,
        };
        if let Some(idx) = parse_index(name) {
            return self.set_property_index(val, idx, value).is_ok();
        }
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

    pub fn set_property_atom_id(&mut self, val: Value, atom: u32, value: Value) -> bool {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
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
            self.atoms.add_ref(id);
            return Some(id);
        }
        let header = self.alloc_string(bytes)?;
        let id = self.atoms.push(AtomEntry { bytes: header, ref_count: 1 })?;
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

    pub fn atom_dup(&mut self, id: u32) -> bool {
        self.atoms.add_ref(id)
    }

    pub fn atom_free(&mut self, id: u32) {
        self.atoms.release(id);
    }

    pub fn object_keys(&self, val: Value) -> Option<Vec<String>> {
        let obj = self.object_ptr(val)?;
        let mut keys = Vec::new();
        unsafe {
            if (*obj).tag == HEAP_TAG_ARRAY {
                let len = (*obj).array_len as usize;
                for i in 0..len {
                    keys.push(i.to_string());
                }
            }
            let mut cur = (*obj).prop_head;
            while !cur.is_null() {
                if (*cur).key_kind == PROP_KEY_INDEX {
                    keys.push((*cur).key.to_string());
                } else {
                    let atom = (*cur).key;
                    if let Some(bytes) = self.atom_bytes(atom) {
                        if let Ok(s) = core::str::from_utf8(bytes) {
                            keys.push(s.to_string());
                        }
                    }
                }
                cur = (*cur).next;
            }
        }
        Some(keys)
    }

    pub fn set_object_proto(&mut self, val: Value, proto: Value) -> bool {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
            None => return false,
        };
        unsafe {
            (*obj).proto = proto;
        }
        true
    }

    pub fn object_proto(&self, val: Value) -> Option<Value> {
        let obj = self.object_ptr(val)?;
        unsafe { Some((*obj).proto) }
    }

    pub fn array_push(&mut self, val: Value, elem: Value) -> Option<u32> {
        let obj = self.object_ptr(val)?;
        unsafe {
            if (*obj).tag != HEAP_TAG_ARRAY {
                return None;
            }
            let idx = (*obj).array_len;
            let _ = self.array_set(obj, idx, elem).ok()?;
            Some((*obj).array_len)
        }
    }

    pub fn array_pop(&mut self, val: Value) -> Option<Value> {
        let obj = self.object_ptr(val)?;
        unsafe {
            if (*obj).tag != HEAP_TAG_ARRAY {
                return None;
            }
            let len = (*obj).array_len as usize;
            if len == 0 {
                return Some(Value::UNDEFINED);
            }
            let idx = (len - 1) as u32;
            let v = self.array_get(obj, idx);
            (*obj).array_len = idx;
            Some(v)
        }
    }

    pub fn alloc_float(&mut self, value: f64) -> Option<*mut u8> {
        let raw = self
            .mem
            .alloc(core::mem::size_of::<FloatHeader>(), core::mem::align_of::<f64>())?;
        self.gc_objects.push(raw);
        unsafe {
            let header = raw as *mut FloatHeader;
            (*header).tag = HEAP_TAG_FLOAT;
            (*header)._pad = 0;
            (*header).value = value;
            Some(raw)
        }
    }

    pub fn float_value(&self, val: Value) -> Option<f64> {
        if !val.is_ptr() {
            return None;
        }
        let ptr = val.as_ptr() as *const FloatHeader;
        unsafe {
            if (*ptr).tag != HEAP_TAG_FLOAT {
                return None;
            }
            Some((*ptr).value)
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

    fn used(&self) -> (usize, usize) {
        (self.offset, self.size)
    }
}

#[allow(dead_code)]
fn _value_size_check(_v: Value) {
    // Ensures the type is used while we bootstrap the runtime.
}

const HEAP_TAG_STRING: u32 = 1;
const HEAP_TAG_OBJECT: u32 = 2;
const HEAP_TAG_ARRAY: u32 = 3;
const HEAP_TAG_FLOAT: u32 = 4;

const PROP_KEY_ATOM: u32 = 0;
const PROP_KEY_INDEX: u32 = 1;

#[repr(C)]
struct StringHeader {
    tag: u32,
    len: u32,
}

#[repr(C)]
struct FloatHeader {
    tag: u32,
    _pad: u32,
    value: f64,
}

fn align_up(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }
    (value + align - 1) & !(align - 1)
}

fn parse_index(name: &[u8]) -> Option<u32> {
    if name.is_empty() {
        return None;
    }
    let mut value: u32 = 0;
    for &b in name {
        if b < b'0' || b > b'9' {
            return None;
        }
        let digit = (b - b'0') as u32;
        value = value.checked_mul(10)?.checked_add(digit)?;
    }
    Some(value)
}

struct AtomTable {
    entries: Vec<AtomEntry>,
}

impl AtomTable {
    fn new() -> Self {
        let mut table = Self { entries: Vec::new() };
        table.entries.push(AtomEntry { bytes: core::ptr::null_mut(), ref_count: 1 });
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

    fn get_mut(&mut self, id: u32) -> Option<&mut AtomEntry> {
        self.entries.get_mut(id as usize)
    }

    fn mark_live(&self, stack: &mut Vec<Value>) {
        for entry in &self.entries {
            if entry.ref_count > 0 && !entry.bytes.is_null() {
                stack.push(Value::from_ptr(entry.bytes));
            }
        }
    }

    fn add_ref(&mut self, id: u32) -> bool {
        if let Some(entry) = self.get_mut(id) {
            entry.ref_count = entry.ref_count.saturating_add(1);
            true
        } else {
            false
        }
    }

    fn release(&mut self, id: u32) {
        if let Some(entry) = self.get_mut(id) {
            if entry.ref_count > 0 {
                entry.ref_count -= 1;
            }
        }
    }
}

struct AtomEntry {
    bytes: *mut u8,
    ref_count: u32,
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
    proto: Value,
    prop_head: *mut Property,
    prop_count: u32,
    array_len: u32,
    array_cap: u32,
    elements: *mut Value,
    opaque: *mut core::ffi::c_void,
    func_idx: i32,
    func_params: Value,
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
        self.gc_objects.push(raw);
        unsafe {
            let obj = raw as *mut HeapObject;
            let tag = if class_id == JSObjectClassEnum::Array as u32 {
                HEAP_TAG_ARRAY
            } else {
                HEAP_TAG_OBJECT
            };
            (*obj).tag = tag;
            (*obj).class_id = class_id;
            (*obj).proto = Value::NULL;
            (*obj).prop_head = core::ptr::null_mut();
            (*obj).prop_count = 0;
            (*obj).array_len = 0;
            (*obj).array_cap = 0;
            (*obj).elements = core::ptr::null_mut();
            (*obj).opaque = core::ptr::null_mut();
            (*obj).func_idx = 0;
            (*obj).func_params = Value::UNDEFINED;
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
        None
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
        // Add at tail to maintain insertion order
        if (*obj).prop_head.is_null() {
            (*obj).prop_head = prop;
        } else {
            let mut tail = (*obj).prop_head;
            while !(*tail).next.is_null() {
                tail = (*tail).next;
            }
            (*tail).next = prop;
        }
        (*obj).prop_count = (*obj).prop_count.saturating_add(1);
        true
    }

    unsafe fn delete_prop_value(&mut self, obj: *mut HeapObject, kind: u32, key: u32) -> bool {
        let mut prev: *mut Property = core::ptr::null_mut();
        let mut cur = (*obj).prop_head;
        while !cur.is_null() {
            if (*cur).key_kind == kind && (*cur).key == key {
                // Found it - unlink from list
                if prev.is_null() {
                    (*obj).prop_head = (*cur).next;
                } else {
                    (*prev).next = (*cur).next;
                }
                (*obj).prop_count = (*obj).prop_count.saturating_sub(1);
                return true;
            }
            prev = cur;
            cur = (*cur).next;
        }
        false
    }

    pub fn delete_property_str(&mut self, val: Value, name: &[u8]) -> bool {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
            None => return false,
        };
        if let Some(idx) = parse_index(name) {
            return self.delete_property_index(val, idx);
        }
        let atom = match self.intern_string(name) {
            Some(atom) => atom,
            None => return false,
        };
        unsafe { self.delete_prop_value(obj, PROP_KEY_ATOM, atom) }
    }

    pub fn delete_property_index(&mut self, val: Value, idx: u32) -> bool {
        let obj = match self.object_ptr(val) {
            Some(obj) => obj,
            None => return false,
        };
        unsafe {
            // For arrays, we can't truly delete indexed elements, but we can undefine them
            if (*obj).tag == HEAP_TAG_ARRAY && idx < (*obj).array_len {
                if !(*obj).elements.is_null() {
                    *(*obj).elements.add(idx as usize) = Value::UNDEFINED;
                }
                return true;
            }
            self.delete_prop_value(obj, PROP_KEY_INDEX, idx)
        }
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
        let idx_usize = idx as usize;
        if idx_usize >= len {
            let new_len = idx_usize + 1;
            if new_len > (*obj).array_cap as usize {
                self.array_grow(obj, new_len)?;
            }
            if (*obj).elements.is_null() {
                return Err(());
            }
            // Fill holes with undefined
            for i in len..idx_usize {
                *(*obj).elements.add(i) = Value::UNDEFINED;
            }
            *(*obj).elements.add(idx_usize) = value;
            (*obj).array_len = new_len as u32;
            return Ok(());
        }
        if (*obj).elements.is_null() {
            return Err(());
        }
        *(*obj).elements.add(idx_usize) = value;
        Ok(())
    }

    unsafe fn array_set_length(&mut self, obj: *mut HeapObject, value: Value) -> Result<(), ()> {
        let new_len = match value.int32() {
            Some(v) if v >= 0 => v as usize,
            _ => return Err(()),
        };
        let current = (*obj).array_len as usize;
        if new_len > current {
            if new_len > (*obj).array_cap as usize {
                self.array_grow(obj, new_len)?;
            }
            if !(*obj).elements.is_null() {
                for i in current..new_len {
                    *(*obj).elements.add(i) = Value::UNDEFINED;
                }
            }
        } else if new_len < current {
            if !(*obj).elements.is_null() {
                for i in new_len..current {
                    *(*obj).elements.add(i) = Value::UNDEFINED;
                }
            }
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

    fn gc_clear_marks(&mut self) {
        self.gc_marks.clear();
    }

    fn gc_mark_value(&mut self, val: Value, stack: &mut Vec<Value>) {
        if !val.is_ptr() {
            return;
        }
        let ptr = val.as_ptr();
        if ptr.is_null() {
            return;
        }
        if self.gc_marks.iter().any(|&p| p == ptr) {
            return;
        }
        self.gc_marks.push(ptr);
        let tag = unsafe { *(ptr as *const u32) };
        match tag {
            HEAP_TAG_STRING => {
                // Strings have no children.
            }
            HEAP_TAG_FLOAT => {
                // Floats have no children.
            }
            HEAP_TAG_OBJECT | HEAP_TAG_ARRAY => {
                let obj = ptr as *mut HeapObject;
                unsafe {
                    stack.push((*obj).proto);
                    if !(*obj).func_params.is_undefined() {
                        stack.push((*obj).func_params);
                    }
                    let mut cur = (*obj).prop_head;
                    while !cur.is_null() {
                        stack.push((*cur).value);
                        cur = (*cur).next;
                    }
                    if (*obj).tag == HEAP_TAG_ARRAY && !(*obj).elements.is_null() {
                        let len = (*obj).array_len as usize;
                        for i in 0..len {
                            stack.push(*(*obj).elements.add(i));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn gc_mark_gcref(&mut self, stack: &mut Vec<Value>) {
        let mut cur = self.gcref_head;
        while !cur.is_null() {
            unsafe {
                stack.push((*cur).val);
                cur = (*cur).prev;
            }
        }
    }
}
