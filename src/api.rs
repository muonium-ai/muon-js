#![allow(non_snake_case)]

use crate::context::Context;
use crate::types::*;
use crate::value::Value;

/// Opaque handle to a VM instance.
pub type JSContextImpl = Context;

/// Create a new context with a caller-provided memory buffer.
/// This mirrors JS_NewContext in mquickjs.h and must stay API-compatible.
pub fn js_new_context(mem: &mut [u8]) -> JSContextImpl {
    Context::new(mem)
}

/// Free the context. Finalizers should run; no system allocator is used.
pub fn js_free_context(_ctx: JSContextImpl) {
    // Placeholder until GC/finalizers are implemented.
}

// --- API stubs mirroring mquickjs.h ---

pub fn js_push_gcref(_ctx: &mut JSContextImpl, _ref: &mut JSGCRef) -> *mut JSValue {
    _ref.prev = _ctx.gcref_head();
    _ctx.set_gcref_head(_ref as *mut JSGCRef);
    &_ref.val as *const JSValue as *mut JSValue
}

pub fn js_pop_gcref(_ctx: &mut JSContextImpl, _ref: &mut JSGCRef) -> JSValue {
    if _ctx.gcref_head() == (_ref as *mut JSGCRef) {
        _ctx.set_gcref_head(_ref.prev);
    }
    _ref.val
}

pub fn js_add_gcref(_ctx: &mut JSContextImpl, _ref: &mut JSGCRef) -> *mut JSValue {
    _ref.prev = _ctx.gcref_head();
    _ctx.set_gcref_head(_ref as *mut JSGCRef);
    &_ref.val as *const JSValue as *mut JSValue
}

pub fn js_delete_gcref(_ctx: &mut JSContextImpl, _ref: &mut JSGCRef) {
    let target = _ref as *mut JSGCRef;
    let mut cur = _ctx.gcref_head();
    let mut prev: *mut JSGCRef = core::ptr::null_mut();

    unsafe {
        while !cur.is_null() {
            if cur == target {
                let next = (*cur).prev;
                if prev.is_null() {
                    _ctx.set_gcref_head(next);
                } else {
                    (*prev).prev = next;
                }
                break;
            }
            prev = cur;
            cur = (*cur).prev;
        }
    }
}

pub fn js_new_context2(mem: &mut [u8], _prepare_compilation: JSBool) -> JSContextImpl {
    Context::new(mem)
}

pub fn js_new_float64(_ctx: &mut JSContextImpl, _d: f64) -> JSValue {
    Value::UNDEFINED
}

pub fn js_new_int32(_ctx: &mut JSContextImpl, _val: i32) -> JSValue {
    Value::from_int32(_val)
}

pub fn js_new_uint32(_ctx: &mut JSContextImpl, _val: u32) -> JSValue {
    Value::from_int32(_val as i32)
}

pub fn js_new_int64(_ctx: &mut JSContextImpl, _val: i64) -> JSValue {
    Value::from_int32(_val as i32)
}

pub fn js_is_number(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    if _val.is_number() { 1 } else { 0 }
}

pub fn js_is_bool(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    if _val.is_bool() { 1 } else { 0 }
}

pub fn js_is_null(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    if _val.is_null() { 1 } else { 0 }
}

pub fn js_is_undefined(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    if _val.is_undefined() { 1 } else { 0 }
}

pub fn js_is_string(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    if _ctx.string_bytes(_val).is_some() { 1 } else { 0 }
}

pub fn js_is_error(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    0
}

pub fn js_is_function(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    0
}

pub fn js_get_class_id(_ctx: &mut JSContextImpl, _val: JSValue) -> i32 {
    0
}

pub fn js_set_opaque(_ctx: &mut JSContextImpl, _val: JSValue, _opaque: *mut core::ffi::c_void) {}
pub fn js_get_opaque(_ctx: &mut JSContextImpl, _val: JSValue) -> *mut core::ffi::c_void {
    core::ptr::null_mut()
}

pub fn js_set_context_opaque(_ctx: &mut JSContextImpl, _opaque: *mut core::ffi::c_void) {
    _ctx.set_opaque(_opaque);
}
pub fn js_set_interrupt_handler(_ctx: &mut JSContextImpl, _handler: Option<JSInterruptHandler>) {
    _ctx.set_interrupt_handler(_handler);
}
pub fn js_set_random_seed(_ctx: &mut JSContextImpl, _seed: u64) {
    _ctx.set_random_seed(_seed);
}

pub fn js_get_global_object(_ctx: &mut JSContextImpl) -> JSValue {
    Value::UNDEFINED
}

pub fn js_throw(_ctx: &mut JSContextImpl, obj: JSValue) -> JSValue {
    let _ = obj;
    Value::EXCEPTION
}

pub fn js_throw_error(_ctx: &mut JSContextImpl, _error_num: JSObjectClassEnum, _msg: &str) -> JSValue {
    Value::EXCEPTION
}

pub fn js_throw_out_of_memory(_ctx: &mut JSContextImpl) -> JSValue {
    Value::EXCEPTION
}

pub fn js_get_property_str(_ctx: &mut JSContextImpl, _this_obj: JSValue, _str: &str) -> JSValue {
    Value::UNDEFINED
}

pub fn js_get_property_uint32(_ctx: &mut JSContextImpl, _obj: JSValue, _idx: u32) -> JSValue {
    Value::UNDEFINED
}

pub fn js_set_property_str(
    _ctx: &mut JSContextImpl,
    _this_obj: JSValue,
    _str: &str,
    _val: JSValue,
) -> JSValue {
    Value::UNDEFINED
}

pub fn js_set_property_uint32(
    _ctx: &mut JSContextImpl,
    _this_obj: JSValue,
    _idx: u32,
    _val: JSValue,
) -> JSValue {
    Value::UNDEFINED
}

pub fn js_new_object_class_user(_ctx: &mut JSContextImpl, _class_id: i32) -> JSValue {
    Value::UNDEFINED
}

pub fn js_new_object(_ctx: &mut JSContextImpl) -> JSValue {
    Value::UNDEFINED
}

pub fn js_new_array(_ctx: &mut JSContextImpl, _initial_len: i32) -> JSValue {
    Value::UNDEFINED
}

pub fn js_new_c_function_params(
    _ctx: &mut JSContextImpl,
    _func_idx: i32,
    _params: JSValue,
) -> JSValue {
    Value::UNDEFINED
}

pub fn js_parse(
    _ctx: &mut JSContextImpl,
    _input: &str,
    _filename: &str,
    _eval_flags: i32,
) -> JSValue {
    Value::UNDEFINED
}

pub fn js_run(_ctx: &mut JSContextImpl, _val: JSValue) -> JSValue {
    Value::UNDEFINED
}

pub fn js_eval(
    _ctx: &mut JSContextImpl,
    _input: &str,
    _filename: &str,
    _eval_flags: i32,
) -> JSValue {
    Value::UNDEFINED
}

pub fn js_gc(_ctx: &mut JSContextImpl) {}

pub fn js_new_string_len(_ctx: &mut JSContextImpl, _buf: &[u8]) -> JSValue {
    if let Some(header) = _ctx.alloc_string(_buf) {
        Value::from_ptr(header)
    } else {
        Value::EXCEPTION
    }
}

pub fn js_new_string(_ctx: &mut JSContextImpl, _buf: &str) -> JSValue {
    js_new_string_len(_ctx, _buf.as_bytes())
}

pub fn js_new_atom(_ctx: &mut JSContextImpl, _buf: &[u8]) -> i32 {
    if let Some(id) = _ctx.intern_string(_buf) {
        id as i32
    } else {
        -1
    }
}

pub fn js_to_cstring_len<'a>(
    _ctx: &'a mut JSContextImpl,
    _val: JSValue,
    _buf: &'a mut JSCStringBuf,
) -> &'a str {
    if let Some(bytes) = _ctx.string_bytes(_val) {
        if let Ok(s) = core::str::from_utf8(bytes) {
            return s;
        }
    }
    ""
}

pub fn js_to_cstring<'a>(
    _ctx: &'a mut JSContextImpl,
    _val: JSValue,
    _buf: &'a mut JSCStringBuf,
) -> &'a str {
    js_to_cstring_len(_ctx, _val, _buf)
}

pub fn js_to_string(_ctx: &mut JSContextImpl, _val: JSValue) -> JSValue {
    if _val.is_int() {
        let mut buf = [0u8; 12];
        let bytes = int_to_decimal_bytes(_val.int32().unwrap_or(0), &mut buf);
        return js_new_string_len(_ctx, bytes);
    }
    if _ctx.string_bytes(_val).is_some() {
        return _val;
    }
    Value::UNDEFINED
}

pub fn js_to_int32(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<i32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v)
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_to_uint32(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<u32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v as u32)
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_to_int32_sat(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<i32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v)
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_to_number(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<f64, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v as f64)
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_get_exception(_ctx: &mut JSContextImpl) -> JSValue {
    Value::UNDEFINED
}

pub fn js_stack_check(_ctx: &mut JSContextImpl, _len: u32) -> i32 {
    0
}

pub fn js_push_arg(_ctx: &mut JSContextImpl, _val: JSValue) {}

pub fn js_call(_ctx: &mut JSContextImpl, _call_flags: i32) -> JSValue {
    Value::UNDEFINED
}

pub fn js_is_bytecode(_buf: &[u8]) -> JSBool {
    0
}

pub fn js_relocate_bytecode(_ctx: &mut JSContextImpl, _buf: &mut [u8]) -> i32 {
    -1
}

pub fn js_load_bytecode(_ctx: &mut JSContextImpl, _buf: &[u8]) -> JSValue {
    Value::UNDEFINED
}

pub fn js_set_log_func(_ctx: &mut JSContextImpl, _write_func: Option<JSWriteFunc>) {
    _ctx.set_log_func(_write_func);
}

pub fn js_print_value(_ctx: &mut JSContextImpl, _val: JSValue) {}

pub fn js_print_value_f(_ctx: &mut JSContextImpl, _val: JSValue, _flags: i32) {}

pub fn js_dump_value_f(_ctx: &mut JSContextImpl, _str: &str, _val: JSValue, _flags: i32) {}

pub fn js_dump_value(_ctx: &mut JSContextImpl, _str: &str, _val: JSValue) {}

pub fn js_dump_memory(_ctx: &mut JSContextImpl, _is_long: JSBool) {}

// --- C-API style aliases for compatibility ---

pub fn JS_NewContext(mem: &mut [u8]) -> JSContextImpl {
    js_new_context(mem)
}

pub fn JS_NewContext2(mem: &mut [u8], prepare_compilation: JSBool) -> JSContextImpl {
    js_new_context2(mem, prepare_compilation)
}

pub fn JS_PushGCRef(ctx: &mut JSContextImpl, r: &mut JSGCRef) -> *mut JSValue {
    js_push_gcref(ctx, r)
}

pub fn JS_PopGCRef(ctx: &mut JSContextImpl, r: &mut JSGCRef) -> JSValue {
    js_pop_gcref(ctx, r)
}

pub fn JS_AddGCRef(ctx: &mut JSContextImpl, r: &mut JSGCRef) -> *mut JSValue {
    js_add_gcref(ctx, r)
}

pub fn JS_DeleteGCRef(ctx: &mut JSContextImpl, r: &mut JSGCRef) {
    js_delete_gcref(ctx, r)
}

pub fn JS_FreeContext(ctx: JSContextImpl) {
    js_free_context(ctx)
}

pub fn JS_NewFloat64(ctx: &mut JSContextImpl, d: f64) -> JSValue {
    js_new_float64(ctx, d)
}

pub fn JS_NewInt32(ctx: &mut JSContextImpl, val: i32) -> JSValue {
    js_new_int32(ctx, val)
}

pub fn JS_NewUint32(ctx: &mut JSContextImpl, val: u32) -> JSValue {
    js_new_uint32(ctx, val)
}

pub fn JS_NewInt64(ctx: &mut JSContextImpl, val: i64) -> JSValue {
    js_new_int64(ctx, val)
}

pub fn JS_IsNumber(ctx: &mut JSContextImpl, val: JSValue) -> JSBool {
    js_is_number(ctx, val)
}

pub fn JS_IsBool(ctx: &mut JSContextImpl, val: JSValue) -> JSBool {
    js_is_bool(ctx, val)
}

pub fn JS_IsNull(ctx: &mut JSContextImpl, val: JSValue) -> JSBool {
    js_is_null(ctx, val)
}

pub fn JS_IsUndefined(ctx: &mut JSContextImpl, val: JSValue) -> JSBool {
    js_is_undefined(ctx, val)
}

pub fn JS_IsString(ctx: &mut JSContextImpl, val: JSValue) -> JSBool {
    js_is_string(ctx, val)
}

pub fn JS_IsError(ctx: &mut JSContextImpl, val: JSValue) -> JSBool {
    js_is_error(ctx, val)
}

pub fn JS_IsFunction(ctx: &mut JSContextImpl, val: JSValue) -> JSBool {
    js_is_function(ctx, val)
}

pub fn JS_GetClassID(ctx: &mut JSContextImpl, val: JSValue) -> i32 {
    js_get_class_id(ctx, val)
}

pub fn JS_SetOpaque(ctx: &mut JSContextImpl, val: JSValue, opaque: *mut core::ffi::c_void) {
    js_set_opaque(ctx, val, opaque)
}

pub fn JS_GetOpaque(ctx: &mut JSContextImpl, val: JSValue) -> *mut core::ffi::c_void {
    js_get_opaque(ctx, val)
}

pub fn JS_SetContextOpaque(ctx: &mut JSContextImpl, opaque: *mut core::ffi::c_void) {
    js_set_context_opaque(ctx, opaque)
}

pub fn JS_SetInterruptHandler(ctx: &mut JSContextImpl, handler: Option<JSInterruptHandler>) {
    js_set_interrupt_handler(ctx, handler)
}

pub fn JS_SetRandomSeed(ctx: &mut JSContextImpl, seed: u64) {
    js_set_random_seed(ctx, seed)
}

pub fn JS_GetGlobalObject(ctx: &mut JSContextImpl) -> JSValue {
    js_get_global_object(ctx)
}

pub fn JS_Throw(ctx: &mut JSContextImpl, obj: JSValue) -> JSValue {
    js_throw(ctx, obj)
}

pub fn JS_ThrowError(ctx: &mut JSContextImpl, error_num: JSObjectClassEnum, msg: &str) -> JSValue {
    js_throw_error(ctx, error_num, msg)
}

pub fn JS_ThrowTypeError(ctx: &mut JSContextImpl, msg: &str) -> JSValue {
    js_throw_error(ctx, JSObjectClassEnum::TypeError, msg)
}

pub fn JS_ThrowReferenceError(ctx: &mut JSContextImpl, msg: &str) -> JSValue {
    js_throw_error(ctx, JSObjectClassEnum::ReferenceError, msg)
}

pub fn JS_ThrowInternalError(ctx: &mut JSContextImpl, msg: &str) -> JSValue {
    js_throw_error(ctx, JSObjectClassEnum::InternalError, msg)
}

pub fn JS_ThrowRangeError(ctx: &mut JSContextImpl, msg: &str) -> JSValue {
    js_throw_error(ctx, JSObjectClassEnum::RangeError, msg)
}

pub fn JS_ThrowSyntaxError(ctx: &mut JSContextImpl, msg: &str) -> JSValue {
    js_throw_error(ctx, JSObjectClassEnum::SyntaxError, msg)
}

pub fn JS_ThrowOutOfMemory(ctx: &mut JSContextImpl) -> JSValue {
    js_throw_out_of_memory(ctx)
}

pub fn JS_GetPropertyStr(ctx: &mut JSContextImpl, this_obj: JSValue, name: &str) -> JSValue {
    js_get_property_str(ctx, this_obj, name)
}

pub fn JS_GetPropertyUint32(ctx: &mut JSContextImpl, obj: JSValue, idx: u32) -> JSValue {
    js_get_property_uint32(ctx, obj, idx)
}

pub fn JS_SetPropertyStr(ctx: &mut JSContextImpl, this_obj: JSValue, name: &str, val: JSValue) -> JSValue {
    js_set_property_str(ctx, this_obj, name, val)
}

pub fn JS_SetPropertyUint32(ctx: &mut JSContextImpl, this_obj: JSValue, idx: u32, val: JSValue) -> JSValue {
    js_set_property_uint32(ctx, this_obj, idx, val)
}

pub fn JS_NewObjectClassUser(ctx: &mut JSContextImpl, class_id: i32) -> JSValue {
    js_new_object_class_user(ctx, class_id)
}

pub fn JS_NewObject(ctx: &mut JSContextImpl) -> JSValue {
    js_new_object(ctx)
}

pub fn JS_NewArray(ctx: &mut JSContextImpl, initial_len: i32) -> JSValue {
    js_new_array(ctx, initial_len)
}

pub fn JS_NewCFunctionParams(ctx: &mut JSContextImpl, func_idx: i32, params: JSValue) -> JSValue {
    js_new_c_function_params(ctx, func_idx, params)
}

pub fn JS_Parse(ctx: &mut JSContextImpl, input: &str, filename: &str, eval_flags: i32) -> JSValue {
    js_parse(ctx, input, filename, eval_flags)
}

pub fn JS_Run(ctx: &mut JSContextImpl, val: JSValue) -> JSValue {
    js_run(ctx, val)
}

pub fn JS_Eval(ctx: &mut JSContextImpl, input: &str, filename: &str, eval_flags: i32) -> JSValue {
    js_eval(ctx, input, filename, eval_flags)
}

pub fn JS_GC(ctx: &mut JSContextImpl) {
    js_gc(ctx)
}

pub fn JS_NewStringLen(ctx: &mut JSContextImpl, buf: &[u8]) -> JSValue {
    js_new_string_len(ctx, buf)
}

pub fn JS_NewString(ctx: &mut JSContextImpl, buf: &str) -> JSValue {
    js_new_string(ctx, buf)
}

pub fn JS_NewAtom(ctx: &mut JSContextImpl, buf: &[u8]) -> i32 {
    js_new_atom(ctx, buf)
}

pub fn JS_ToCStringLen<'a>(
    ctx: &'a mut JSContextImpl,
    val: JSValue,
    buf: &'a mut JSCStringBuf,
) -> &'a str {
    js_to_cstring_len(ctx, val, buf)
}

pub fn JS_ToCString<'a>(
    ctx: &'a mut JSContextImpl,
    val: JSValue,
    buf: &'a mut JSCStringBuf,
) -> &'a str {
    js_to_cstring(ctx, val, buf)
}

pub fn JS_ToString(ctx: &mut JSContextImpl, val: JSValue) -> JSValue {
    js_to_string(ctx, val)
}

pub fn JS_ToInt32(ctx: &mut JSContextImpl, val: JSValue) -> Result<i32, JSValue> {
    js_to_int32(ctx, val)
}

pub fn JS_ToUint32(ctx: &mut JSContextImpl, val: JSValue) -> Result<u32, JSValue> {
    js_to_uint32(ctx, val)
}

pub fn JS_ToInt32Sat(ctx: &mut JSContextImpl, val: JSValue) -> Result<i32, JSValue> {
    js_to_int32_sat(ctx, val)
}

pub fn JS_ToNumber(ctx: &mut JSContextImpl, val: JSValue) -> Result<f64, JSValue> {
    js_to_number(ctx, val)
}

pub fn JS_GetException(ctx: &mut JSContextImpl) -> JSValue {
    js_get_exception(ctx)
}

pub fn JS_StackCheck(ctx: &mut JSContextImpl, len: u32) -> i32 {
    js_stack_check(ctx, len)
}

pub fn JS_PushArg(ctx: &mut JSContextImpl, val: JSValue) {
    js_push_arg(ctx, val)
}

pub fn JS_Call(ctx: &mut JSContextImpl, call_flags: i32) -> JSValue {
    js_call(ctx, call_flags)
}

pub fn JS_IsBytecode(buf: &[u8]) -> JSBool {
    js_is_bytecode(buf)
}

pub fn JS_RelocateBytecode(ctx: &mut JSContextImpl, buf: &mut [u8]) -> i32 {
    js_relocate_bytecode(ctx, buf)
}

pub fn JS_LoadBytecode(ctx: &mut JSContextImpl, buf: &[u8]) -> JSValue {
    js_load_bytecode(ctx, buf)
}

pub fn JS_SetLogFunc(ctx: &mut JSContextImpl, write_func: Option<JSWriteFunc>) {
    js_set_log_func(ctx, write_func)
}

pub fn JS_PrintValue(ctx: &mut JSContextImpl, val: JSValue) {
    js_print_value(ctx, val)
}

pub fn JS_PrintValueF(ctx: &mut JSContextImpl, val: JSValue, flags: i32) {
    js_print_value_f(ctx, val, flags)
}

pub fn JS_DumpValueF(ctx: &mut JSContextImpl, label: &str, val: JSValue, flags: i32) {
    js_dump_value_f(ctx, label, val, flags)
}

pub fn JS_DumpValue(ctx: &mut JSContextImpl, label: &str, val: JSValue) {
    js_dump_value(ctx, label, val)
}

pub fn JS_DumpMemory(ctx: &mut JSContextImpl, is_long: JSBool) {
    js_dump_memory(ctx, is_long)
}

fn int_to_decimal_bytes(mut value: i32, buf: &mut [u8; 12]) -> &[u8] {
    if value == 0 {
        buf[0] = b'0';
        return &buf[0..1];
    }
    let negative = value < 0;
    if negative {
        value = -value;
    }
    let mut idx = buf.len();
    while value > 0 {
        let digit = (value % 10) as u8;
        value /= 10;
        idx -= 1;
        buf[idx] = b'0' + digit;
    }
    if negative {
        idx -= 1;
        buf[idx] = b'-';
    }
    &buf[idx..]
}
