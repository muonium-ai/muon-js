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
    if let Some(ptr) = _ctx.alloc_float(_d) {
        Value::from_ptr(ptr)
    } else {
        js_throw_out_of_memory(_ctx)
    }
}

pub fn js_new_int32(_ctx: &mut JSContextImpl, _val: i32) -> JSValue {
    Value::from_int32(_val)
}

pub fn js_new_uint32(_ctx: &mut JSContextImpl, _val: u32) -> JSValue {
    if _val <= i32::MAX as u32 {
        Value::from_int32(_val as i32)
    } else if let Some(ptr) = _ctx.alloc_float(_val as f64) {
        Value::from_ptr(ptr)
    } else {
        js_throw_out_of_memory(_ctx)
    }
}

pub fn js_new_int64(_ctx: &mut JSContextImpl, _val: i64) -> JSValue {
    if _val >= i32::MIN as i64 && _val <= i32::MAX as i64 {
        Value::from_int32(_val as i32)
    } else if let Some(ptr) = _ctx.alloc_float(_val as f64) {
        Value::from_ptr(ptr)
    } else {
        js_throw_out_of_memory(_ctx)
    }
}

pub fn js_is_number(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    if _val.is_number() || _ctx.float_value(_val).is_some() { 1 } else { 0 }
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
    match _ctx.object_class_id(_val) {
        Some(id) => {
            let min = JSObjectClassEnum::Error as u32;
            let max = JSObjectClassEnum::InternalError as u32;
            if id >= min && id <= max { 1 } else { 0 }
        }
        None => 0,
    }
}

pub fn js_is_function(_ctx: &mut JSContextImpl, _val: JSValue) -> JSBool {
    match _ctx.object_class_id(_val) {
        Some(id) => {
            let func = JSObjectClassEnum::CFunction as u32;
            let closure = JSObjectClassEnum::Closure as u32;
            if id == func || id == closure { 1 } else { 0 }
        }
        None => 0,
    }
}

pub fn js_get_class_id(_ctx: &mut JSContextImpl, _val: JSValue) -> i32 {
    _ctx.object_class_id(_val).map(|v| v as i32).unwrap_or(-1)
}

pub fn js_set_opaque(_ctx: &mut JSContextImpl, _val: JSValue, _opaque: *mut core::ffi::c_void) {
    _ctx.set_object_opaque(_val, _opaque);
}
pub fn js_get_opaque(_ctx: &mut JSContextImpl, _val: JSValue) -> *mut core::ffi::c_void {
    _ctx.get_object_opaque(_val)
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
    _ctx.global_object()
}

pub fn js_throw(_ctx: &mut JSContextImpl, obj: JSValue) -> JSValue {
    _ctx.set_exception(obj);
    Value::EXCEPTION
}

pub fn js_throw_error(_ctx: &mut JSContextImpl, _error_num: JSObjectClassEnum, _msg: &str) -> JSValue {
    let msg = js_new_string(_ctx, _msg);
    _ctx.set_exception(msg);
    Value::EXCEPTION
}

pub fn js_throw_out_of_memory(_ctx: &mut JSContextImpl) -> JSValue {
    _ctx.set_exception(Value::UNDEFINED);
    Value::EXCEPTION
}

pub fn js_get_property_str(_ctx: &mut JSContextImpl, _this_obj: JSValue, _str: &str) -> JSValue {
    _ctx.get_property_str(_this_obj, _str.as_bytes()).unwrap_or(Value::UNDEFINED)
}

pub fn js_get_property_uint32(_ctx: &mut JSContextImpl, _obj: JSValue, _idx: u32) -> JSValue {
    _ctx.get_property_index(_obj, _idx).unwrap_or(Value::UNDEFINED)
}

pub fn js_set_property_str(
    _ctx: &mut JSContextImpl,
    _this_obj: JSValue,
    _str: &str,
    _val: JSValue,
) -> JSValue {
    if _ctx.set_property_str(_this_obj, _str.as_bytes(), _val) {
        _val
    } else {
        js_throw_error(_ctx, JSObjectClassEnum::TypeError, "property set failed")
    }
}

pub fn js_set_property_uint32(
    _ctx: &mut JSContextImpl,
    _this_obj: JSValue,
    _idx: u32,
    _val: JSValue,
) -> JSValue {
    match _ctx.set_property_index(_this_obj, _idx, _val) {
        Ok(()) => _val,
        Err(()) => js_throw_error(_ctx, JSObjectClassEnum::TypeError, "array index out of bounds"),
    }
}

pub fn js_new_object_class_user(_ctx: &mut JSContextImpl, _class_id: i32) -> JSValue {
    _ctx
        .new_object(_class_id as u32)
        .unwrap_or_else(|| js_throw_out_of_memory(_ctx))
}

pub fn js_new_object(_ctx: &mut JSContextImpl) -> JSValue {
    _ctx
        .new_object(JSObjectClassEnum::Object as u32)
        .unwrap_or_else(|| js_throw_out_of_memory(_ctx))
}

pub fn js_new_array(_ctx: &mut JSContextImpl, _initial_len: i32) -> JSValue {
    if _initial_len < 0 {
        return Value::EXCEPTION;
    }
    _ctx
        .new_array(_initial_len as usize)
        .unwrap_or_else(|| js_throw_out_of_memory(_ctx))
}

pub fn js_new_c_function_params(
    _ctx: &mut JSContextImpl,
    _func_idx: i32,
    _params: JSValue,
) -> JSValue {
    _ctx
        .new_c_function(_func_idx, _params)
        .unwrap_or(Value::EXCEPTION)
}

pub fn js_parse(
    _ctx: &mut JSContextImpl,
    _input: &str,
    _filename: &str,
    _eval_flags: i32,
) -> JSValue {
    js_new_string(_ctx, _input)
}

pub fn js_run(_ctx: &mut JSContextImpl, _val: JSValue) -> JSValue {
    if let Some(bytes) = _ctx.string_bytes(_val) {
        if let Ok(src) = core::str::from_utf8(bytes) {
            let owned = src.to_owned();
            return js_eval(_ctx, &owned, "<run>", 0);
        }
    }
    Value::EXCEPTION
}

pub fn js_eval(
    _ctx: &mut JSContextImpl,
    _input: &str,
    _filename: &str,
    _eval_flags: i32,
) -> JSValue {
    let src = _input.trim();
    if let Some(val) = eval_expr(_ctx, src) {
        return val;
    }
    Value::EXCEPTION
}

pub fn js_gc(_ctx: &mut JSContextImpl) {}

pub fn js_new_string_len(_ctx: &mut JSContextImpl, _buf: &[u8]) -> JSValue {
    if let Some(header) = _ctx.alloc_string(_buf) {
        Value::from_ptr(header)
    } else {
        js_throw_out_of_memory(_ctx)
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
    let mut val = _val;
    if _ctx.string_bytes(val).is_none() {
        val = js_to_string(_ctx, val);
    }
    if let Some(bytes) = _ctx.string_bytes(val) {
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
    if let Some(f) = _ctx.float_value(_val) {
        let s = f.to_string();
        return js_new_string(_ctx, &s);
    }
    if _ctx.string_bytes(_val).is_some() {
        return _val;
    }
    if _val.is_bool() {
        if _val == Value::TRUE {
            return js_new_string_len(_ctx, b"true");
        }
        return js_new_string_len(_ctx, b"false");
    }
    if _val.is_null() {
        return js_new_string_len(_ctx, b"null");
    }
    if _val.is_undefined() {
        return js_new_string_len(_ctx, b"undefined");
    }
    if let Some(class_id) = _ctx.object_class_id(_val) {
        if class_id == JSObjectClassEnum::Array as u32 {
            return js_new_string(_ctx, "[object Array]");
        }
        return js_new_string(_ctx, "[object Object]");
    }
    Value::UNDEFINED
}

pub fn js_to_int32(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<i32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v)
    } else if let Some(f) = _ctx.float_value(_val) {
        Ok(f as i32)
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_to_uint32(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<u32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v as u32)
    } else if let Some(f) = _ctx.float_value(_val) {
        Ok(f as u32)
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_to_int32_sat(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<i32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v)
    } else if let Some(f) = _ctx.float_value(_val) {
        if f.is_nan() {
            Ok(0)
        } else if f > i32::MAX as f64 {
            Ok(i32::MAX)
        } else if f < i32::MIN as f64 {
            Ok(i32::MIN)
        } else {
            Ok(f as i32)
        }
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_to_number(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<f64, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v as f64)
    } else if let Some(f) = _ctx.float_value(_val) {
        Ok(f)
    } else if _val.is_bool() {
        Ok(if _val == Value::TRUE { 1.0 } else { 0.0 })
    } else if _val.is_null() {
        Ok(0.0)
    } else {
        Err(Value::EXCEPTION)
    }
}

pub fn js_get_exception(_ctx: &mut JSContextImpl) -> JSValue {
    _ctx.get_exception()
}

pub fn js_stack_check(_ctx: &mut JSContextImpl, _len: u32) -> i32 {
    _ctx.stack_check(_len)
}

pub fn js_push_arg(_ctx: &mut JSContextImpl, _val: JSValue) {
    _ctx.push_arg(_val);
}

pub fn js_call(_ctx: &mut JSContextImpl, _call_flags: i32) -> JSValue {
    _ctx.call(_call_flags)
}

pub fn js_is_bytecode(_buf: &[u8]) -> JSBool {
    if _buf.len() < 2 {
        return 0;
    }
    let magic = u16::from_ne_bytes([_buf[0], _buf[1]]);
    if magic == JS_BYTECODE_MAGIC { 1 } else { 0 }
}

pub fn js_relocate_bytecode(_ctx: &mut JSContextImpl, _buf: &mut [u8]) -> i32 {
    if js_is_bytecode(_buf) != 0 { 0 } else { -1 }
}

pub fn js_load_bytecode(_ctx: &mut JSContextImpl, _buf: &[u8]) -> JSValue {
    if js_is_bytecode(_buf) != 0 {
        Value::UNDEFINED
    } else {
        Value::EXCEPTION
    }
}

pub fn js_set_log_func(_ctx: &mut JSContextImpl, _write_func: Option<JSWriteFunc>) {
    _ctx.set_log_func(_write_func);
}

pub fn js_print_value(_ctx: &mut JSContextImpl, _val: JSValue) {
    let mut buf = JSCStringBuf { buf: [0u8; 5] };
    let owned = {
        let s = js_to_cstring(_ctx, _val, &mut buf);
        s.as_bytes().to_vec()
    };
    _ctx.write_log(&owned);
    _ctx.write_log(b"\n");
}

pub fn js_print_value_f(_ctx: &mut JSContextImpl, _val: JSValue, _flags: i32) {
    js_print_value(_ctx, _val);
}

pub fn js_dump_value_f(_ctx: &mut JSContextImpl, _str: &str, _val: JSValue, _flags: i32) {
    _ctx.write_log(_str.as_bytes());
    _ctx.write_log(b": ");
    let mut buf = JSCStringBuf { buf: [0u8; 5] };
    let owned = {
        let s = js_to_cstring(_ctx, _val, &mut buf);
        s.as_bytes().to_vec()
    };
    _ctx.write_log(&owned);
    _ctx.write_log(b"\n");
}

pub fn js_dump_value(_ctx: &mut JSContextImpl, _str: &str, _val: JSValue) {
    js_dump_value_f(_ctx, _str, _val, 0);
}

pub fn js_dump_memory(_ctx: &mut JSContextImpl, _is_long: JSBool) {
    let (used, size) = _ctx.memory_usage();
    let mut buf = [0u8; 64];
    let mut idx = 0;
    idx += write_decimal(&mut buf[idx..], used);
    buf[idx] = b'/';
    idx += 1;
    idx += write_decimal(&mut buf[idx..], size);
    buf[idx] = b'\n';
    idx += 1;
    _ctx.write_log(&buf[..idx]);
}

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

fn write_decimal(buf: &mut [u8], mut value: usize) -> usize {
    if value == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut tmp = [0u8; 20];
    let mut idx = tmp.len();
    while value > 0 {
        let digit = (value % 10) as u8;
        value /= 10;
        idx -= 1;
        tmp[idx] = b'0' + digit;
    }
    let len = tmp.len() - idx;
    let out_len = len.min(buf.len());
    buf[..out_len].copy_from_slice(&tmp[idx..idx + out_len]);
    out_len
}

fn number_to_value(ctx: &mut JSContextImpl, val: f64) -> JSValue {
    if !val.is_finite() {
        ctx.set_exception(Value::UNDEFINED);
        return Value::EXCEPTION;
    }
    if val.fract() == 0.0 && val >= i32::MIN as f64 && val <= i32::MAX as f64 {
        return Value::from_int32(val as i32);
    }
    if let Some(ptr) = ctx.alloc_float(val) {
        Value::from_ptr(ptr)
    } else {
        ctx.set_exception(Value::UNDEFINED);
        Value::EXCEPTION
    }
}

fn parse_numeric_expr(src: &str) -> Result<f64, ()> {
    let mut parser = ExprParser::new(src.as_bytes());
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return Err(());
    }
    Ok(value)
}

fn eval_value(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    if s.starts_with('[') && s.ends_with(']') {
        return eval_array_literal(ctx, s);
    }
    if s.starts_with('{') && s.ends_with('}') {
        return eval_object_literal(ctx, s);
    }
    if s == "null" {
        return Some(Value::NULL);
    }
    if s == "undefined" {
        return Some(Value::UNDEFINED);
    }
    if s == "true" {
        return Some(Value::TRUE);
    }
    if s == "false" {
        return Some(Value::FALSE);
    }
    if (s.starts_with('\"') && s.ends_with('\"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        let inner = &s[1..s.len() - 1];
        return Some(js_new_string(ctx, inner));
    }
    if let Ok(num) = parse_numeric_expr(s) {
        return Some(number_to_value(ctx, num));
    }
    if s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        return eval_expr(ctx, inner);
    }
    None
}

fn eval_expr(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    if s.is_empty() {
        return None;
    }
    let (base, tail) = split_base_and_tail(s)?;
    let mut val = eval_value(ctx, base)?;
    let mut rest = tail;
    loop {
        let rest_trim = rest.trim_start();
        if rest_trim.is_empty() {
            return Some(val);
        }
        if rest_trim.starts_with('.') {
            let (name, next) = parse_identifier(&rest_trim[1..])?;
            val = js_get_property_str(ctx, val, name);
            rest = next;
            continue;
        }
        if rest_trim.starts_with('[') {
            let (inside, next) = extract_bracket(rest_trim)?;
            let idx_val = eval_expr(ctx, inside)?;
            if let Some(i) = idx_val.int32() {
                val = js_get_property_uint32(ctx, val, i as u32);
            } else if let Some(bytes) = ctx.string_bytes(idx_val) {
                let owned = bytes.to_vec();
                let name = core::str::from_utf8(&owned).ok()?;
                val = js_get_property_str(ctx, val, name);
            } else {
                return None;
            }
            rest = next;
            continue;
        }
        return None;
    }
}

fn split_base_and_tail(src: &str) -> Option<(&str, &str)> {
    let s = src.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if b == string_delim {
                in_string = false;
            }
            continue;
        }
        if b == b'\'' || b == b'\"' {
            in_string = true;
            string_delim = b;
            continue;
        }
        if depth == 0 && b == b'.' {
            let next = bytes.get(i + 1).copied();
            if next.map(is_ident_start).unwrap_or(false) {
                let base = s[..i].trim();
                let tail = &s[i..];
                if base.is_empty() {
                    return None;
                }
                return Some((base, tail));
            }
        }
        if b == b'[' {
            if depth == 0 && i > 0 {
                let base = s[..i].trim();
                let tail = &s[i..];
                if base.is_empty() {
                    return None;
                }
                return Some((base, tail));
            }
            depth += 1;
            continue;
        }
        match b {
            b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => depth -= 1,
            _ => {}
        }
    }
    Some((s, ""))
}

fn extract_bracket(src: &str) -> Option<(&str, &str)> {
    let bytes = src.as_bytes();
    if bytes.first().copied() != Some(b'[') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    for i in 0..bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == string_delim {
                in_string = false;
            }
            continue;
        }
        if b == b'\'' || b == b'\"' {
            in_string = true;
            string_delim = b;
            continue;
        }
        if b == b'[' {
            depth += 1;
            continue;
        }
        if b == b']' {
            depth -= 1;
            if depth == 0 {
                let inside = &src[1..i];
                let rest = &src[i + 1..];
                return Some((inside, rest));
            }
        }
    }
    None
}

fn parse_identifier(src: &str) -> Option<(&str, &str)> {
    let bytes = src.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    if !is_ident_start(bytes[0]) {
        return None;
    }
    let mut end = 1usize;
    for b in &bytes[1..] {
        let ok = (b'A'..=b'Z').contains(b)
            || (b'a'..=b'z').contains(b)
            || (b'0'..=b'9').contains(b)
            || *b == b'_';
        if !ok {
            break;
        }
        end += 1;
    }
    Some((&src[..end], &src[end..]))
}

fn is_ident_start(b: u8) -> bool {
    (b'A'..=b'Z').contains(&b) || (b'a'..=b'z').contains(&b) || b == b'_'
}

fn eval_array_literal(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let inner = src.trim();
    let inner = &inner[1..inner.len().saturating_sub(1)];
    let items = split_top_level(inner)?;
    let arr = js_new_array(ctx, items.len() as i32);
    if arr.is_exception() {
        return None;
    }
    for (idx, item) in items.iter().enumerate() {
        let val = eval_value(ctx, item)?;
        let res = js_set_property_uint32(ctx, arr, idx as u32, val);
        if res.is_exception() {
            return None;
        }
    }
    Some(arr)
}

fn eval_object_literal(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let inner = src.trim();
    let inner = &inner[1..inner.len().saturating_sub(1)];
    let entries = split_top_level(inner)?;
    let obj = js_new_object(ctx);
    if obj.is_exception() {
        return None;
    }
    for entry in entries {
        let mut parts = entry.splitn(2, ':');
        let key = parts.next()?.trim();
        let value_src = parts.next()?.trim();
        let key_str = if (key.starts_with('\"') && key.ends_with('\"') && key.len() >= 2)
            || (key.starts_with('\'') && key.ends_with('\'') && key.len() >= 2)
        {
            &key[1..key.len() - 1]
        } else {
            key
        };
        let val = eval_value(ctx, value_src)?;
        let res = js_set_property_str(ctx, obj, key_str, val);
        if res.is_exception() {
            return None;
        }
    }
    Some(obj)
}

fn split_top_level(src: &str) -> Option<Vec<&str>> {
    let s = src.trim();
    if s.is_empty() {
        return Some(Vec::new());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if b == string_delim {
                in_string = false;
            }
            continue;
        }
        if b == b'\'' || b == b'\"' {
            in_string = true;
            string_delim = b;
            continue;
        }
        if b == b'[' || b == b'{' || b == b'(' {
            depth += 1;
            continue;
        }
        if b == b']' || b == b'}' || b == b')' {
            depth -= 1;
            if depth < 0 {
                return None;
            }
            continue;
        }
        if b == b',' && depth == 0 {
            let part = s[start..i].trim();
            out.push(part);
            start = i + 1;
        }
    }
    if depth != 0 {
        return None;
    }
    let part = s[start..].trim();
    if !part.is_empty() {
        out.push(part);
    }
    Some(out)
}

struct ExprParser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> ExprParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    fn parse_expr(&mut self) -> Result<f64, ()> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(b'+') => b'+',
                Some(b'-') => b'-',
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_term()?;
            if op == b'+' {
                value += rhs;
            } else {
                value -= rhs;
            }
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<f64, ()> {
        let mut value = self.parse_factor()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(b'*') => b'*',
                Some(b'/') => b'/',
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_factor()?;
            if op == b'*' {
                value *= rhs;
            } else {
                value /= rhs;
            }
        }
        Ok(value)
    }

    fn parse_factor(&mut self) -> Result<f64, ()> {
        self.skip_ws();
        if let Some(b'+') = self.peek() {
            self.pos += 1;
            return self.parse_factor();
        }
        if let Some(b'-') = self.peek() {
            self.pos += 1;
            return Ok(-self.parse_factor()?);
        }
        if let Some(b'(') = self.peek() {
            self.pos += 1;
            let value = self.parse_expr()?;
            self.skip_ws();
            if self.peek() != Some(b')') {
                return Err(());
            }
            self.pos += 1;
            return Ok(value);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Result<f64, ()> {
        self.skip_ws();
        let start = self.pos;
        let mut saw_digit = false;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                saw_digit = true;
                self.pos += 1;
            } else {
                break;
            }
        }
        if let Some(b'.') = self.peek() {
            self.pos += 1;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    saw_digit = true;
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        if !saw_digit {
            return Err(());
        }
        let slice = &self.input[start..self.pos];
        let s = core::str::from_utf8(slice).map_err(|_| ())?;
        s.parse::<f64>().map_err(|_| ())
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
}
