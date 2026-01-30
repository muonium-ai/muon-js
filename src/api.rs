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

pub fn js_new_context_with_stdlib(
    mem: &mut [u8],
    stdlib_def: Option<&JSSTDLibraryDef>,
    cfunc_len: usize,
) -> JSContextImpl {
    let mut ctx = Context::new(mem);
    if let Some(def) = stdlib_def {
        js_set_stdlib_def(&mut ctx, def, cfunc_len);
    }
    ctx
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
    if (_eval_flags & JS_EVAL_JSON) != 0 {
        if let Some(val) = parse_json(_ctx, _input) {
            return val;
        }
        return js_throw_error(_ctx, JSObjectClassEnum::SyntaxError, "invalid JSON");
    }
    js_new_string(_ctx, _input)
}

pub fn js_run(_ctx: &mut JSContextImpl, _val: JSValue) -> JSValue {
    if let Some(bytes) = _ctx.string_bytes(_val) {
        if let Ok(src) = core::str::from_utf8(bytes) {
            let owned = src.to_owned();
            return js_eval(_ctx, &owned, "<run>", JS_EVAL_RETVAL);
        }
    }
    if _val.is_exception() {
        return _val;
    }
    _val
}

pub fn js_eval(
    _ctx: &mut JSContextImpl,
    _input: &str,
    _filename: &str,
    _eval_flags: i32,
) -> JSValue {
    let src = _input.trim();
    if (_eval_flags & JS_EVAL_JSON) != 0 {
        if let Some(val) = parse_json(_ctx, src) {
            return val;
        }
        return js_throw_error(_ctx, JSObjectClassEnum::SyntaxError, "invalid JSON");
    }
    if let Some(val) = eval_program(_ctx, src) {
        if val.is_exception() {
            return val;
        }
        if (_eval_flags & JS_EVAL_RETVAL) != 0 {
            return val;
        }
        return Value::UNDEFINED;
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
        if f.is_finite() { Ok(f as i32) } else { Ok(0) }
    } else {
        let n = js_to_number(_ctx, _val)?;
        if !n.is_finite() {
            Ok(0)
        } else {
            Ok(n as i32)
        }
    }
}

pub fn js_to_uint32(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<u32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v as u32)
    } else if let Some(f) = _ctx.float_value(_val) {
        if f.is_finite() { Ok(f as u32) } else { Ok(0) }
    } else {
        let n = js_to_number(_ctx, _val)?;
        if !n.is_finite() {
            Ok(0)
        } else {
            Ok(n as u32)
        }
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
        let n = js_to_number(_ctx, _val)?;
        if n.is_nan() {
            Ok(0)
        } else if n > i32::MAX as f64 {
            Ok(i32::MAX)
        } else if n < i32::MIN as f64 {
            Ok(i32::MIN)
        } else {
            Ok(n as i32)
        }
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
    } else if _val.is_undefined() {
        Ok(f64::NAN)
    } else if let Some(bytes) = _ctx.string_bytes(_val) {
        if let Ok(s) = core::str::from_utf8(bytes) {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(0.0);
            }
            match trimmed {
                "Infinity" | "+Infinity" => return Ok(f64::INFINITY),
                "-Infinity" => return Ok(f64::NEG_INFINITY),
                "NaN" => return Ok(f64::NAN),
                _ => {}
            }
            if let Some(rest) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
                if let Ok(v) = u64::from_str_radix(rest, 16) {
                    return Ok(v as f64);
                }
            }
            if let Some(rest) = trimmed.strip_prefix("0o").or_else(|| trimmed.strip_prefix("0O")) {
                if let Ok(v) = u64::from_str_radix(rest, 8) {
                    return Ok(v as f64);
                }
            }
            if let Some(rest) = trimmed.strip_prefix("0b").or_else(|| trimmed.strip_prefix("0B")) {
                if let Ok(v) = u64::from_str_radix(rest, 2) {
                    return Ok(v as f64);
                }
            }
            return Ok(trimmed.parse::<f64>().unwrap_or(f64::NAN));
        }
        Ok(f64::NAN)
    } else if _ctx.object_class_id(_val).is_some() {
        Ok(f64::NAN)
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
    let argc = (_call_flags & 0xffff) as usize;
    let need = argc + 2;
    if _ctx.stack_check(need as u32) != 0 {
        return js_throw_error(_ctx, JSObjectClassEnum::InternalError, "stack overflow");
    }
    let stack_len = _ctx.call_stack_len();
    if stack_len < need {
        return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "stack underflow");
    }
    let this_idx = stack_len - 1;
    let func_idx = stack_len - 2;
    let func_val = _ctx.call_stack_get(func_idx);
    let _this_val = _ctx.call_stack_get(this_idx);
    let mut args = Vec::with_capacity(argc);
    for i in 0..argc {
        let arg = _ctx.call_stack_get(stack_len - 3 - i);
        args.push(arg);
    }
    _ctx.call_stack_truncate(stack_len - need);
    if let Some((idx, params)) = _ctx.c_function_info(func_val) {
        return call_c_function(_ctx, idx, params, _this_val, &args);
    }
    js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not a function")
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

pub fn js_set_c_function_table(_ctx: &mut JSContextImpl, table: &[JSCFunctionDef]) {
    _ctx.set_c_function_table(table.as_ptr(), table.len());
}

pub fn js_set_stdlib_def(_ctx: &mut JSContextImpl, def: &JSSTDLibraryDef, cfunc_len: usize) {
    _ctx.set_c_function_table(def.c_function_table, cfunc_len);
}

pub fn js_register_global_function(
    _ctx: &mut JSContextImpl,
    name: &str,
    func_idx: i32,
    params: JSValue,
) -> JSValue {
    let func = js_new_c_function_params(_ctx, func_idx, params);
    let global = js_get_global_object(_ctx);
    let res = js_set_property_str(_ctx, global, name, func);
    if res.is_exception() {
        return res;
    }
    func
}

pub fn js_register_stdlib_minimal(_ctx: &mut JSContextImpl) -> JSValue {
    let obj_ctor = js_new_c_function_params(_ctx, 0, JSValue::UNDEFINED);
    let arr_ctor = js_new_c_function_params(_ctx, 1, JSValue::UNDEFINED);
    let global = js_get_global_object(_ctx);
    let _ = js_set_property_str(_ctx, global, "Object", obj_ctor);
    let _ = js_set_property_str(_ctx, global, "Array", arr_ctor);
    let object_proto = js_new_object(_ctx);
    let _ = js_set_property_str(_ctx, obj_ctor, "prototype", object_proto);
    _ctx.set_object_proto_default(object_proto);
    let _ = _ctx.set_object_proto(global, object_proto);
    let array_proto = js_new_object(_ctx);
    let _ = _ctx.set_object_proto(array_proto, object_proto);
    let _ = js_set_property_str(_ctx, arr_ctor, "prototype", array_proto);
    _ctx.set_array_proto(array_proto);
    if _ctx.c_function_def(2).is_some() {
        let keys_fn = js_new_c_function_params(_ctx, 2, JSValue::UNDEFINED);
        let _ = js_set_property_str(_ctx, obj_ctor, "keys", keys_fn);
    }
    if _ctx.c_function_def(3).is_some() {
        let is_array_fn = js_new_c_function_params(_ctx, 3, JSValue::UNDEFINED);
        let _ = js_set_property_str(_ctx, arr_ctor, "isArray", is_array_fn);
    }
    if _ctx.c_function_def(4).is_some() {
        let create_fn = js_new_c_function_params(_ctx, 4, JSValue::UNDEFINED);
        let _ = js_set_property_str(_ctx, obj_ctor, "create", create_fn);
    }
    if _ctx.c_function_def(7).is_some() {
        let define_fn = js_new_c_function_params(_ctx, 7, JSValue::UNDEFINED);
        let _ = js_set_property_str(_ctx, obj_ctor, "defineProperty", define_fn);
    }
    if _ctx.c_function_def(10).is_some() {
        let get_proto_fn = js_new_c_function_params(_ctx, 10, JSValue::UNDEFINED);
        let _ = js_set_property_str(_ctx, obj_ctor, "getPrototypeOf", get_proto_fn);
    }
    if _ctx.c_function_def(8).is_some() || _ctx.c_function_def(9).is_some() {
        let mut push_val = Value::UNDEFINED;
        let mut pop_val = Value::UNDEFINED;
        if _ctx.c_function_def(8).is_some() {
            let push_fn = js_new_c_function_params(_ctx, 8, JSValue::UNDEFINED);
            let _ = js_set_property_str(_ctx, array_proto, "push", push_fn);
            push_val = push_fn;
        }
        if _ctx.c_function_def(9).is_some() {
            let pop_fn = js_new_c_function_params(_ctx, 9, JSValue::UNDEFINED);
            let _ = js_set_property_str(_ctx, array_proto, "pop", pop_fn);
            pop_val = pop_fn;
        }
        _ctx.set_array_proto_methods(push_val, pop_val);
    }
    if _ctx.c_function_def(5).is_some() {
        let math = js_new_object(_ctx);
        let _ = js_set_property_str(_ctx, global, "Math", math);
        if let Some(def) = _ctx.c_function_def(5) {
            if def.def_type == JSCFunctionDefEnum::FF as u8 {
                let abs_fn = js_new_c_function_params(_ctx, 5, JSValue::UNDEFINED);
                let _ = js_set_property_str(_ctx, math, "abs", abs_fn);
            }
        }
        if let Some(def) = _ctx.c_function_def(6) {
            if def.def_type == JSCFunctionDefEnum::FF as u8 {
                let floor_fn = js_new_c_function_params(_ctx, 6, JSValue::UNDEFINED);
                let _ = js_set_property_str(_ctx, math, "floor", floor_fn);
            }
        }
        if let Some(def) = _ctx.c_function_def(15) {
            if def.def_type == JSCFunctionDefEnum::FF as u8 {
                let ceil_fn = js_new_c_function_params(_ctx, 15, JSValue::UNDEFINED);
                let _ = js_set_property_str(_ctx, math, "ceil", ceil_fn);
            }
        }
        if let Some(def) = _ctx.c_function_def(16) {
            if def.def_type == JSCFunctionDefEnum::FF as u8 {
                let trunc_fn = js_new_c_function_params(_ctx, 16, JSValue::UNDEFINED);
                let _ = js_set_property_str(_ctx, math, "trunc", trunc_fn);
            }
        }
        if let Some(def) = _ctx.c_function_def(17) {
            if def.def_type == JSCFunctionDefEnum::FF as u8 {
                let round_fn = js_new_c_function_params(_ctx, 17, JSValue::UNDEFINED);
                let _ = js_set_property_str(_ctx, math, "round", round_fn);
            }
        }
    }
    if _ctx.c_function_def(11).is_some() {
        let date = js_new_object(_ctx);
        let _ = js_set_property_str(_ctx, global, "Date", date);
        let now_fn = js_new_c_function_params(_ctx, 11, JSValue::UNDEFINED);
        let _ = js_set_property_str(_ctx, date, "now", now_fn);
    }
    Value::UNDEFINED
}

pub fn js_object_keys(_ctx: &mut JSContextImpl, obj: JSValue) -> JSValue {
    let keys = match _ctx.object_keys(obj) {
        Some(keys) => keys,
        None => return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not an object"),
    };
    let arr = js_new_array(_ctx, keys.len() as i32);
    if arr.is_exception() {
        return arr;
    }
    for (i, key) in keys.iter().enumerate() {
        let s = js_new_string(_ctx, key);
        let _ = js_set_property_uint32(_ctx, arr, i as u32, s);
    }
    arr
}

pub fn js_array_is_array(_ctx: &mut JSContextImpl, val: JSValue) -> JSValue {
    if _ctx.object_class_id(val) == Some(JSObjectClassEnum::Array as u32) {
        Value::TRUE
    } else {
        Value::FALSE
    }
}

pub fn js_object_create(_ctx: &mut JSContextImpl, proto: JSValue) -> JSValue {
    if !proto.is_null() && _ctx.object_class_id(proto).is_none() {
        return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "invalid prototype");
    }
    let obj = js_new_object(_ctx);
    if obj.is_exception() {
        return obj;
    }
    let _ = _ctx.set_object_proto(obj, proto);
    obj
}

pub fn js_object_define_property(_ctx: &mut JSContextImpl, obj: JSValue, key: JSValue, val: JSValue) -> JSValue {
    if _ctx.object_class_id(obj).is_none() {
        return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not an object");
    }
    if let Some(bytes) = _ctx.string_bytes(key) {
        let owned = bytes.to_vec();
        if let Ok(name) = core::str::from_utf8(&owned) {
            let res = js_set_property_str(_ctx, obj, name, val);
            if res.is_exception() {
                return res;
            }
            return obj;
        }
    }
    if let Some(i) = key.int32() {
        let res = js_set_property_uint32(_ctx, obj, i as u32, val);
        if res.is_exception() {
            return res;
        }
        return obj;
    }
    js_throw_error(_ctx, JSObjectClassEnum::TypeError, "invalid property key")
}

pub fn js_object_get_prototype_of(_ctx: &mut JSContextImpl, obj: JSValue) -> JSValue {
    if _ctx.object_class_id(obj).is_none() {
        return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not an object");
    }
    match _ctx.object_proto(obj) {
        Some(proto) if !proto.is_undefined() => proto,
        _ => Value::NULL,
    }
}

pub fn js_array_push(_ctx: &mut JSContextImpl, arr: JSValue, elem: JSValue) -> JSValue {
    match _ctx.array_push(arr, elem) {
        Some(len) => Value::from_int32(len as i32),
        None => js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not an array"),
    }
}

pub fn js_array_pop(_ctx: &mut JSContextImpl, arr: JSValue) -> JSValue {
    match _ctx.array_pop(arr) {
        Some(val) => val,
        None => js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not an array"),
    }
}

pub fn js_date_now(_ctx: &mut JSContextImpl) -> JSValue {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    match now {
        Ok(dur) => {
            let ms = dur.as_millis() as f64;
            js_new_float64(_ctx, ms)
        }
        Err(_) => js_throw_error(_ctx, JSObjectClassEnum::InternalError, "time error"),
    }
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

pub fn JS_NewContextWithStdlib(
    mem: &mut [u8],
    stdlib_def: Option<&JSSTDLibraryDef>,
    cfunc_len: usize,
) -> JSContextImpl {
    js_new_context_with_stdlib(mem, stdlib_def, cfunc_len)
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

pub fn JS_SetCFunctionTable(ctx: &mut JSContextImpl, table: &[JSCFunctionDef]) {
    js_set_c_function_table(ctx, table)
}

pub fn JS_SetStdlibDef(ctx: &mut JSContextImpl, def: &JSSTDLibraryDef, cfunc_len: usize) {
    js_set_stdlib_def(ctx, def, cfunc_len)
}

pub fn JS_RegisterGlobalFunction(
    ctx: &mut JSContextImpl,
    name: &str,
    func_idx: i32,
    params: JSValue,
) -> JSValue {
    js_register_global_function(ctx, name, func_idx, params)
}

pub fn JS_RegisterStdlibMinimal(ctx: &mut JSContextImpl) -> JSValue {
    js_register_stdlib_minimal(ctx)
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

fn parse_arith_expr(ctx: &mut JSContextImpl, src: &str) -> Result<JSValue, ()> {
    let mut parser = ArithParser::new(ctx, src.as_bytes());
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return Err(());
    }
    Ok(value)
}

fn contains_arith_op(src: &str) -> bool {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut last_was_operand = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == string_delim {
                in_string = false;
                last_was_operand = true;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'\"' {
            in_string = true;
            string_delim = b;
            i += 1;
            continue;
        }
        match b {
            b'[' | b'{' | b'(' => {
                depth += 1;
                last_was_operand = false;
            }
            b']' | b'}' | b')' => {
                depth -= 1;
                last_was_operand = true;
            }
            b'+' | b'-' | b'/' | b'%' if depth == 0 && last_was_operand => {
                return true;
            }
            b'*' if depth == 0 && last_was_operand => {
                // Check for ** (exponentiation) or * (multiplication)
                return true;
            }
            b'<' | b'>' if depth == 0 => {
                // Check for <<, >>, <=, >= or simple comparison <, >
                if i + 1 < bytes.len() {
                    let next = bytes[i + 1];
                    // Any of these combinations mean it's an operator
                    if next == b'<' || next == b'>' || next == b'=' {
                        return true;
                    }
                }
                // Simple < or > comparison (with operand before it)
                if last_was_operand {
                    return true;
                }
            }
            b'=' if depth == 0 && last_was_operand => {
                return true;
            }
            b'!' if depth == 0 => {
                // Check for != vs !
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    if last_was_operand {
                        return true;
                    }
                } else if !last_was_operand {
                    // Unary ! at start
                    return true;
                }
            }
            b'~' if depth == 0 && !last_was_operand => {
                // Bitwise NOT is unary
                return true;
            }
            b'&' | b'|' | b'^' if depth == 0 && last_was_operand => {
                return true;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {
                // whitespace doesn't affect operand status
            }
            _ => {
                last_was_operand = true;
            }
        }
        i += 1;
    }
    false
}

fn is_simple_string_literal(src: &str) -> bool {
    let bytes = src.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    let quote = bytes[0];
    if quote != b'\"' && quote != b'\'' {
        return false;
    }
    if bytes[bytes.len() - 1] != quote {
        return false;
    }
    let mut i = 1usize;
    while i + 1 < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == quote {
            return false;
        }
        i += 1;
    }
    true
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
    if s == "Math" {
        // Return a special marker for Math object
        return Some(js_new_string(ctx, "__builtin_Math__"));
    }
    if s == "Object" {
        return Some(js_new_string(ctx, "__builtin_Object__"));
    }
    if s == "Array" {
        return Some(js_new_string(ctx, "__builtin_Array__"));
    }
    if s == "Number" {
        return Some(js_new_string(ctx, "__builtin_Number__"));
    }
    if s == "String" {
        return Some(js_new_string(ctx, "__builtin_String__"));
    }
    if s == "parseInt" {
        return Some(js_new_string(ctx, "__builtin_parseInt__"));
    }
    if s == "parseFloat" {
        return Some(js_new_string(ctx, "__builtin_parseFloat__"));
    }
    if s == "isNaN" {
        return Some(js_new_string(ctx, "__builtin_isNaN__"));
    }
    if s == "isFinite" {
        return Some(js_new_string(ctx, "__builtin_isFinite__"));
    }
    if s == "NaN" {
        return Some(number_to_value(ctx, f64::NAN));
    }
    if s == "Infinity" {
        return Some(number_to_value(ctx, f64::INFINITY));
    }
    if is_simple_string_literal(s) {
        let inner = &s[1..s.len() - 1];
        return Some(js_new_string(ctx, inner));
    }
    if contains_arith_op(s) {
        if let Ok(val) = parse_arith_expr(ctx, s) {
            return Some(val);
        }
    }
    if let Ok(num) = parse_numeric_expr(s) {
        return Some(number_to_value(ctx, num));
    }
    if s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        return eval_expr(ctx, inner);
    }
    if is_identifier(s) {
        let global = js_get_global_object(ctx);
        let v = js_get_property_str(ctx, global, s);
        return Some(v);
    }
    None
}

fn eval_expr(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    if s.is_empty() {
        return None;
    }
    // Check for compound assignment operators: +=, -=, *=, /=
    if s.contains("+=") || s.contains("-=") || s.contains("*=") || s.contains("/=") {
        let bytes = s.as_bytes();
        let mut depth = 0i32;
        let mut in_string = false;
        let mut string_delim = 0u8;
        for i in 1..bytes.len() {
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
            match b {
                b'[' | b'{' | b'(' => depth += 1,
                b']' | b'}' | b')' => depth -= 1,
                b'=' if depth == 0 => {
                    let prev = bytes[i - 1];
                    if prev == b'+' || prev == b'-' || prev == b'*' || prev == b'/' {
                        let lhs = s[..i - 1].trim();
                        let rhs = s[i + 1..].trim();
                        if !lhs.is_empty() && !rhs.is_empty() {
                            // Expand: x += 5 => x = x + 5
                            let op = prev as char;
                            let expanded = format!("{} = {} {} {}", lhs, lhs, op, rhs);
                            return eval_expr(ctx, &expanded);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some((lhs, rhs)) = split_assignment(s) {
        let rhs_val = eval_expr(ctx, rhs)?;
        let (base, key) = parse_lvalue(ctx, lhs)?;
        let res = match key {
            LValueKey::Index(idx) => js_set_property_uint32(ctx, base, idx, rhs_val),
            LValueKey::Name(name) => js_set_property_str(ctx, base, &name, rhs_val),
        };
        if res.is_exception() {
            return None;
        }
        return Some(rhs_val);
    }
    // Check for ternary operator: condition ? true_val : false_val
    if let Some((cond, true_part, false_part)) = split_ternary(s) {
        let cond_val = eval_expr(ctx, cond)?;
        let is_true = is_truthy(cond_val);
        if is_true {
            return eval_expr(ctx, true_part);
        } else {
            return eval_expr(ctx, false_part);
        }
    }
    // Check for arithmetic operators before splitting on base/tail
    if contains_arith_op(s) {
        if let Ok(val) = parse_arith_expr(ctx, s) {
            return Some(val);
        }
    }
    // Check for postfix ++ or --
    if s.ends_with("++") || s.ends_with("--") {
        let var_name = &s[..s.len() - 2].trim();
        if is_identifier(var_name) {
            let global = js_get_global_object(ctx);
            let old_val = js_get_property_str(ctx, global, var_name);
            let n = js_to_number(ctx, old_val).ok()?;
            let new_val = if s.ends_with("++") {
                number_to_value(ctx, n + 1.0)
            } else {
                number_to_value(ctx, n - 1.0)
            };
            js_set_property_str(ctx, global, var_name, new_val);
            return Some(old_val); // postfix returns old value
        }
    }
    // Check for prefix ++ or --
    if s.starts_with("++") || s.starts_with("--") {
        let var_name = &s[2..].trim();
        if is_identifier(var_name) {
            let global = js_get_global_object(ctx);
            let old_val = js_get_property_str(ctx, global, var_name);
            let n = js_to_number(ctx, old_val).ok()?;
            let new_val = if s.starts_with("++") {
                number_to_value(ctx, n + 1.0)
            } else {
                number_to_value(ctx, n - 1.0)
            };
            js_set_property_str(ctx, global, var_name, new_val);
            return Some(new_val); // prefix returns new value
        }
    }
    // Check for typeof operator
    if s.starts_with("typeof ") {
        let operand = s[7..].trim();
        let val = eval_expr(ctx, operand)?;
        let type_str = if val.is_bool() {
            "boolean"
        } else if val.is_number() {
            "number"
        } else if js_is_string(ctx, val) != 0 {
            "string"
        } else if val.is_undefined() {
            "undefined"
        } else if val.is_null() {
            "object"  // typeof null is "object" in JavaScript
        } else if js_is_function(ctx, val) != 0 {
            "function"
        } else if val.is_ptr() {
            "object"
        } else {
            "undefined"
        };
        return Some(js_new_string(ctx, type_str));
    }
    let (base, tail) = split_base_and_tail(s)?;
    let mut val = eval_value(ctx, base)?;
    let mut this_val = Value::UNDEFINED;
    let mut rest = tail;
    loop {
        let rest_trim = rest.trim_start();
        if rest_trim.is_empty() {
            return Some(val);
        }
        if rest_trim.starts_with('(') {
            let (inside, next) = extract_paren(rest_trim)?;
            let arg_list = split_top_level(inside)?;
            let mut args = Vec::new();
            for arg in arg_list {
                if arg.is_empty() {
                    continue;
                }
                let v = eval_expr(ctx, arg)?;
                args.push(v);
            }
            
            // Check if val is a closure (our custom function)
            let closure_marker = js_get_property_str(ctx, val, "__closure__");
            if closure_marker == Value::TRUE {
                val = call_closure(ctx, val, &args)?;
                this_val = Value::UNDEFINED;
                rest = next;
                continue;
            }
            
            // Check for built-in method markers
            if let Some(bytes) = ctx.string_bytes(val) {
                if let Ok(marker) = core::str::from_utf8(bytes) {
                    if marker == "__builtin_array_pop__" {
                        val = js_array_pop(ctx, this_val);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_charAt__" {
                        if args.len() == 1 {
                            if let Some(idx) = args[0].int32() {
                                if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                    if idx >= 0 && (idx as usize) < str_bytes.len() {
                                        let ch = str_bytes[idx as usize];
                                        // Create a vector to own the byte
                                        let mut ch_buf = [0u8; 4];
                                        ch_buf[0] = ch;
                                        let ch_str = core::str::from_utf8(&ch_buf[..1]).unwrap_or("");
                                        val = js_new_string(ctx, ch_str);
                                    } else {
                                        val = js_new_string(ctx, "");
                                    }
                                    this_val = Value::UNDEFINED;
                                    rest = next;
                                    continue;
                                }
                            }
                        }
                    } else if marker == "__builtin_string_substring__" {
                        if args.len() >= 1 && args.len() <= 2 {
                            if let Some(start) = args[0].int32() {
                                if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                    let start = start.max(0) as usize;
                                    let start = start.min(str_bytes.len());
                                    let end = if args.len() == 2 {
                                        if let Some(e) = args[1].int32() {
                                            let e = e.max(0) as usize;
                                            e.min(str_bytes.len())
                                        } else {
                                            str_bytes.len()
                                        }
                                    } else {
                                        str_bytes.len()
                                    };
                                    let (start, end) = if start > end { (end, start) } else { (start, end) };
                                    // Copy the substring to avoid borrow issues
                                    let substr_bytes = str_bytes[start..end].to_vec();
                                    if let Ok(substr) = core::str::from_utf8(&substr_bytes) {
                                        val = js_new_string(ctx, substr);
                                        this_val = Value::UNDEFINED;
                                        rest = next;
                                        continue;
                                    }
                                }
                            }
                        }
                    } else if marker == "__builtin_string_indexOf__" {
                        if args.len() == 1 {
                            if let Some(needle_bytes) = ctx.string_bytes(args[0]) {
                                if let Some(haystack_bytes) = ctx.string_bytes(this_val) {
                                    // Simple substring search
                                    let needle = needle_bytes;
                                    let haystack = haystack_bytes;
                                    if needle.is_empty() {
                                        val = Value::from_int32(0);
                                    } else {
                                        let mut found = -1;
                                        for i in 0..=(haystack.len().saturating_sub(needle.len())) {
                                            if &haystack[i..i + needle.len()] == needle {
                                                found = i as i32;
                                                break;
                                            }
                                        }
                                        val = Value::from_int32(found);
                                    }
                                    this_val = Value::UNDEFINED;
                                    rest = next;
                                    continue;
                                }
                            }
                        }
                    } else if marker == "__builtin_string_slice__" {
                        if args.len() == 2 {
                            if let (Some(start), Some(end)) = (args[0].int32(), args[1].int32()) {
                                if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                    let len = str_bytes.len() as i32;
                                    let start = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
                                    let end = if end < 0 { (len + end).max(0) } else { end.min(len) } as usize;
                                    if start <= end {
                                        // Copy the substring to avoid borrow issues
                                        let substr_bytes = str_bytes[start..end].to_vec();
                                        if let Ok(substr) = core::str::from_utf8(&substr_bytes) {
                                            val = js_new_string(ctx, substr);
                                            this_val = Value::UNDEFINED;
                                            rest = next;
                                            continue;
                                        }
                                    } else {
                                        val = js_new_string(ctx, "");
                                        this_val = Value::UNDEFINED;
                                        rest = next;
                                        continue;
                                    }
                                }
                            }
                        }
                    } else if marker == "__builtin_array_shift__" {
                        // Get first element and remove it
                        let first_elem = js_get_property_uint32(ctx, this_val, 0);
                        // Get array length
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        if let Some(len) = len_val.int32() {
                            if len > 0 {
                                // Shift all elements down
                                for i in 0..(len - 1) {
                                    let elem = js_get_property_uint32(ctx, this_val, (i + 1) as u32);
                                    js_set_property_uint32(ctx, this_val, i as u32, elem);
                                }
                                // Set new length
                                js_set_property_str(ctx, this_val, "length", Value::from_int32(len - 1));
                                val = first_elem;
                            } else {
                                val = Value::UNDEFINED;
                            }
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_unshift__" {
                        if args.len() == 1 {
                            // Get array length
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            if let Some(len) = len_val.int32() {
                                // Shift all elements up
                                for i in (0..len).rev() {
                                    let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                    js_set_property_uint32(ctx, this_val, (i + 1) as u32, elem);
                                }
                                // Set first element
                                js_set_property_uint32(ctx, this_val, 0, args[0]);
                                // Set new length
                                js_set_property_str(ctx, this_val, "length", Value::from_int32(len + 1));
                                val = Value::from_int32(len + 1);
                            } else {
                                val = Value::UNDEFINED;
                            }
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_join__" {
                        // Ported from mquickjs js_array_join (mquickjs.c:14253)
                        // Get separator (default to comma)
                        let separator = if args.len() > 0 && args[0] != Value::UNDEFINED {
                            // Convert separator to string
                            if let Some(sep_bytes) = ctx.string_bytes(args[0]) {
                                if let Ok(sep_str) = core::str::from_utf8(sep_bytes) {
                                    sep_str.to_string()
                                } else {
                                    ",".to_string()
                                }
                            } else if let Some(n) = args[0].int32() {
                                n.to_string()
                            } else {
                                ",".to_string()
                            }
                        } else {
                            ",".to_string()
                        };
                        
                        // Get array length
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = if let Some(n) = len_val.int32() {
                            n.max(0) as u32
                        } else {
                            0
                        };
                        
                        // Build result string
                        let mut result = String::new();
                        for i in 0..len {
                            if i > 0 {
                                result.push_str(&separator);
                            }
                            
                            // Get array element
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            
                            // Skip undefined and null (mquickjs behavior)
                            if elem.is_undefined() || elem.is_null() {
                                continue;
                            }
                            
                            // Convert element to string
                            if let Some(n) = elem.int32() {
                                result.push_str(&n.to_string());
                            } else if let Some(bytes) = ctx.string_bytes(elem) {
                                if let Ok(s) = core::str::from_utf8(bytes) {
                                    result.push_str(s);
                                }
                            } else if elem == Value::TRUE {
                                result.push_str("true");
                            } else if elem == Value::FALSE {
                                result.push_str("false");
                            }
                            // TODO: Add float64 support when available
                        }
                        
                        val = js_new_string(ctx, &result);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_toString__" {
                        // Ported from mquickjs.c:14317-14321 js_array_toString
                        // toString() is just join(',')
                        let separator = ",";
                        
                        // Get array length
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = if let Some(n) = len_val.int32() {
                            n.max(0) as u32
                        } else {
                            0
                        };
                        
                        // Build result string
                        let mut result = String::new();
                        for i in 0..len {
                            if i > 0 {
                                result.push_str(separator);
                            }
                            
                            // Get array element
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            
                            // Skip undefined and null (mquickjs behavior)
                            if elem.is_undefined() || elem.is_null() {
                                continue;
                            }
                            
                            // Convert element to string
                            if let Some(n) = elem.int32() {
                                result.push_str(&n.to_string());
                            } else if let Some(bytes) = ctx.string_bytes(elem) {
                                if let Ok(s) = core::str::from_utf8(bytes) {
                                    result.push_str(s);
                                }
                            } else if elem == Value::TRUE {
                                result.push_str("true");
                            } else if elem == Value::FALSE {
                                result.push_str("false");
                            }
                        }
                        
                        val = js_new_string(ctx, &result);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_reverse__" {
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        if let Some(len) = len_val.int32() {
                            for i in 0..(len / 2) {
                                let left = js_get_property_uint32(ctx, this_val, i as u32);
                                let right = js_get_property_uint32(ctx, this_val, (len - 1 - i) as u32);
                                js_set_property_uint32(ctx, this_val, i as u32, right);
                                js_set_property_uint32(ctx, this_val, (len - 1 - i) as u32, left);
                            }
                            val = this_val;
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_split__" {
                        if args.len() == 1 {
                            if let (Some(str_bytes), Some(sep_bytes)) = (ctx.string_bytes(this_val), ctx.string_bytes(args[0])) {
                                let str_owned = str_bytes.to_vec();
                                let sep_owned = sep_bytes.to_vec();
                                let arr = js_new_array(ctx, 0);
                                if let (Ok(s), Ok(sep)) = (core::str::from_utf8(&str_owned), core::str::from_utf8(&sep_owned)) {
                                    let mut idx = 0u32;
                                    let parts: Vec<&str> = s.split(sep).collect();
                                    for part in parts {
                                        let part_val = js_new_string(ctx, part);
                                        js_set_property_uint32(ctx, arr, idx, part_val);
                                        idx += 1;
                                    }
                                }
                                val = arr;
                            } else {
                                val = js_new_array(ctx, 0);
                            }
                        } else {
                            val = js_new_array(ctx, 0);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_toUpperCase__" {
                        if let Some(str_bytes) = ctx.string_bytes(this_val) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                let upper = s.to_uppercase();
                                val = js_new_string(ctx, &upper);
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_toLowerCase__" {
                        if let Some(str_bytes) = ctx.string_bytes(this_val) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                let lower = s.to_lowercase();
                                val = js_new_string(ctx, &lower);
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_floor__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = Value::from_int32(n.floor() as i32);
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_ceil__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = Value::from_int32(n.ceil() as i32);
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_round__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = Value::from_int32(n.round() as i32);
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_abs__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.abs());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_max__" {
                        if args.len() > 0 {
                            let mut max = f64::NEG_INFINITY;
                            for arg in args {
                                if let Ok(n) = js_to_number(ctx, arg) {
                                    if n > max {
                                        max = n;
                                    }
                                }
                            }
                            val = number_to_value(ctx, max);
                        } else {
                            val = number_to_value(ctx, f64::NEG_INFINITY);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_min__" {
                        if args.len() > 0 {
                            let mut min = f64::INFINITY;
                            for arg in args {
                                if let Ok(n) = js_to_number(ctx, arg) {
                                    if n < min {
                                        min = n;
                                    }
                                }
                            }
                            val = number_to_value(ctx, min);
                        } else {
                            val = number_to_value(ctx, f64::INFINITY);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_sqrt__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.sqrt());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_trim__" {
                        if let Some(str_bytes) = ctx.string_bytes(this_val) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                let trimmed = s.trim().to_string();
                                val = js_new_string(ctx, &trimmed);
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_startsWith__" {
                        if args.len() == 1 {
                            if let (Some(str_bytes), Some(prefix_bytes)) = (ctx.string_bytes(this_val), ctx.string_bytes(args[0])) {
                                if let (Ok(s), Ok(prefix)) = (core::str::from_utf8(str_bytes), core::str::from_utf8(prefix_bytes)) {
                                    val = Value::new_bool(s.starts_with(prefix));
                                } else {
                                    val = Value::FALSE;
                                }
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_endsWith__" {
                        if args.len() == 1 {
                            if let (Some(str_bytes), Some(suffix_bytes)) = (ctx.string_bytes(this_val), ctx.string_bytes(args[0])) {
                                if let (Ok(s), Ok(suffix)) = (core::str::from_utf8(str_bytes), core::str::from_utf8(suffix_bytes)) {
                                    val = Value::new_bool(s.ends_with(suffix));
                                } else {
                                    val = Value::FALSE;
                                }
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_includes__" {
                        if args.len() == 1 {
                            if let (Some(str_bytes), Some(search_bytes)) = (ctx.string_bytes(this_val), ctx.string_bytes(args[0])) {
                                if let (Ok(s), Ok(search)) = (core::str::from_utf8(str_bytes), core::str::from_utf8(search_bytes)) {
                                    val = Value::new_bool(s.contains(search));
                                } else {
                                    val = Value::FALSE;
                                }
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_concat__" {
                        // Ported from mquickjs.c:14347-14395 js_array_concat
                        // Calculate total length needed
                        let this_len_val = js_get_property_str(ctx, this_val, "length");
                        let mut total_len = this_len_val.int32().unwrap_or(0);
                        
                        for arg in &args {
                            if let Some(class_id) = ctx.object_class_id(*arg) {
                                if class_id == JSObjectClassEnum::Array as u32 {
                                    let arg_len_val = js_get_property_str(ctx, *arg, "length");
                                    total_len += arg_len_val.int32().unwrap_or(0);
                                } else {
                                    total_len += 1;
                                }
                            } else {
                                total_len += 1;
                            }
                        }
                        
                        // Create new array and fill it
                        let arr = js_new_array(ctx, total_len);
                        let mut pos = 0u32;
                        
                        // First add this_val (the original array)
                        if let Some(this_len) = this_len_val.int32() {
                            for i in 0..this_len {
                                let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                js_set_property_uint32(ctx, arr, pos, elem);
                                pos += 1;
                            }
                        }
                        
                        // Then add all arguments
                        for arg in &args {
                            if let Some(class_id) = ctx.object_class_id(*arg) {
                                if class_id == JSObjectClassEnum::Array as u32 {
                                    // It's an array - add all elements
                                    let arg_len_val = js_get_property_str(ctx, *arg, "length");
                                    if let Some(arg_len) = arg_len_val.int32() {
                                        for i in 0..arg_len {
                                            let elem = js_get_property_uint32(ctx, *arg, i as u32);
                                            js_set_property_uint32(ctx, arr, pos, elem);
                                            pos += 1;
                                        }
                                    }
                                } else {
                                    // Not an array - add as single element
                                    js_set_property_uint32(ctx, arr, pos, *arg);
                                    pos += 1;
                                }
                            } else {
                                // Not an object - add as single element
                                js_set_property_uint32(ctx, arr, pos, *arg);
                                pos += 1;
                            }
                        }
                        
                        val = arr;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_slice__" {
                        // Ported from mquickjs.c:14440-14477 js_array_slice
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0);
                        
                        // Get start index with proper negative handling
                        let start = if args.len() > 0 {
                            if let Some(start_val) = args[0].int32() {
                                let mut s = start_val;
                                if s < 0 {
                                    s += len;
                                    if s < 0 {
                                        s = 0;
                                    }
                                }
                                s.min(len)
                            } else {
                                len
                            }
                        } else {
                            len
                        };
                        
                        // Get end index with proper negative handling
                        let final_idx = if args.len() > 1 {
                            if let Some(end_val) = args[1].int32() {
                                let mut e = end_val;
                                if e < 0 {
                                    e += len;
                                    if e < 0 {
                                        e = 0;
                                    }
                                }
                                e.min(len)
                            } else {
                                len
                            }
                        } else {
                            len
                        };
                        
                        // Create new array and copy elements
                        let slice_len = (final_idx - start).max(0);
                        let arr = js_new_array(ctx, slice_len);
                        let mut idx = 0u32;
                        for i in start..final_idx {
                            let elem = js_get_property_uint32(ctx, this_val, i as u32);
                            js_set_property_uint32(ctx, arr, idx, elem);
                            idx += 1;
                        }
                        
                        val = arr;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_indexOf__" {
                        if args.len() == 1 {
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            if let Some(len) = len_val.int32() {
                                let search_val = args[0];
                                let mut found_idx = -1;
                                for i in 0..len {
                                    let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                    if elem.0 == search_val.0 {
                                        found_idx = i;
                                        break;
                                    }
                                }
                                val = Value::from_int32(found_idx);
                            } else {
                                val = Value::from_int32(-1);
                            }
                        } else {
                            val = Value::from_int32(-1);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_lastIndexOf__" {
                        if args.len() == 1 {
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            if let Some(len) = len_val.int32() {
                                let search_val = args[0];
                                let mut found_idx = -1;
                                for i in (0..len).rev() {
                                    let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                    if elem.0 == search_val.0 {
                                        found_idx = i;
                                        break;
                                    }
                                }
                                val = Value::from_int32(found_idx);
                            } else {
                                val = Value::from_int32(-1);
                            }
                        } else {
                            val = Value::from_int32(-1);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_fill__" {
                        if args.len() >= 1 {
                            let fill_val = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            if let Some(len) = len_val.int32() {
                                for i in 0..len {
                                    js_set_property_uint32(ctx, this_val, i as u32, fill_val);
                                }
                            }
                            val = this_val;
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_includes__" {
                        if args.len() == 1 {
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            if let Some(len) = len_val.int32() {
                                let search_val = args[0];
                                let mut found = false;
                                for i in 0..len {
                                    let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                    if elem.0 == search_val.0 {
                                        found = true;
                                        break;
                                    }
                                }
                                val = Value::new_bool(found);
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_repeat__" {
                        if args.len() == 1 {
                            if let Some(count) = args[0].int32() {
                                if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                    if let Ok(s) = core::str::from_utf8(str_bytes) {
                                        let repeated = s.repeat(count.max(0) as usize);
                                        val = js_new_string(ctx, &repeated);
                                    } else {
                                        val = this_val;
                                    }
                                } else {
                                    val = this_val;
                                }
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_replace__" {
                        if args.len() >= 2 {
                            if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                if let Ok(s) = core::str::from_utf8(str_bytes) {
                                    if let Some(search_bytes) = ctx.string_bytes(args[0]) {
                                        if let Ok(search) = core::str::from_utf8(search_bytes) {
                                            if let Some(replace_bytes) = ctx.string_bytes(args[1]) {
                                                if let Ok(replace) = core::str::from_utf8(replace_bytes) {
                                                    // Simple replace - only first occurrence
                                                    let result = s.replacen(search, replace, 1);
                                                    val = js_new_string(ctx, &result);
                                                } else {
                                                    val = this_val;
                                                }
                                            } else {
                                                val = this_val;
                                            }
                                        } else {
                                            val = this_val;
                                        }
                                    } else {
                                        val = this_val;
                                    }
                                } else {
                                    val = this_val;
                                }
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_charCodeAt__" {
                        if args.len() >= 1 {
                            if let Some(idx) = args[0].int32() {
                                if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                    if let Ok(s) = core::str::from_utf8(str_bytes) {
                                        if idx >= 0 && (idx as usize) < s.len() {
                                            if let Some(ch) = s.chars().nth(idx as usize) {
                                                val = Value::from_int32(ch as i32);
                                            } else {
                                                val = number_to_value(ctx, f64::NAN);
                                            }
                                        } else {
                                            val = number_to_value(ctx, f64::NAN);
                                        }
                                    } else {
                                        val = number_to_value(ctx, f64::NAN);
                                    }
                                } else {
                                    val = number_to_value(ctx, f64::NAN);
                                }
                            } else {
                                val = number_to_value(ctx, f64::NAN);
                            }
                        } else {
                            val = number_to_value(ctx, f64::NAN);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_trimStart__" {
                        if let Some(str_bytes) = ctx.string_bytes(this_val) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                let trimmed = s.trim_start().to_string();
                                val = js_new_string(ctx, &trimmed);
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_trimEnd__" {
                        if let Some(str_bytes) = ctx.string_bytes(this_val) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                let trimmed = s.trim_end().to_string();
                                val = js_new_string(ctx, &trimmed);
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_padStart__" {
                        if args.len() >= 1 {
                            if let Some(target_len) = args[0].int32() {
                                if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                    if let Ok(s) = core::str::from_utf8(str_bytes) {
                                        let pad_str = if args.len() >= 2 {
                                            if let Some(pad_bytes) = ctx.string_bytes(args[1]) {
                                                core::str::from_utf8(pad_bytes).unwrap_or(" ")
                                            } else {
                                                " "
                                            }
                                        } else {
                                            " "
                                        };
                                        
                                        let current_len = s.len();
                                        if target_len as usize > current_len {
                                            let pad_len = target_len as usize - current_len;
                                            let mut result = String::new();
                                            let pad_str_len = pad_str.len();
                                            if pad_str_len > 0 {
                                                let full_repeats = pad_len / pad_str_len;
                                                let remainder = pad_len % pad_str_len;
                                                for _ in 0..full_repeats {
                                                    result.push_str(pad_str);
                                                }
                                                if remainder > 0 {
                                                    result.push_str(&pad_str[..remainder]);
                                                }
                                            }
                                            result.push_str(s);
                                            val = js_new_string(ctx, &result);
                                        } else {
                                            val = this_val;
                                        }
                                    } else {
                                        val = this_val;
                                    }
                                } else {
                                    val = this_val;
                                }
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_padEnd__" {
                        if args.len() >= 1 {
                            if let Some(target_len) = args[0].int32() {
                                if let Some(str_bytes) = ctx.string_bytes(this_val) {
                                    if let Ok(s) = core::str::from_utf8(str_bytes) {
                                        let pad_str = if args.len() >= 2 {
                                            if let Some(pad_bytes) = ctx.string_bytes(args[1]) {
                                                core::str::from_utf8(pad_bytes).unwrap_or(" ")
                                            } else {
                                                " "
                                            }
                                        } else {
                                            " "
                                        };
                                        
                                        let current_len = s.len();
                                        if target_len as usize > current_len {
                                            let pad_len = target_len as usize - current_len;
                                            let mut result = String::from(s);
                                            let pad_str_len = pad_str.len();
                                            if pad_str_len > 0 {
                                                let full_repeats = pad_len / pad_str_len;
                                                let remainder = pad_len % pad_str_len;
                                                for _ in 0..full_repeats {
                                                    result.push_str(pad_str);
                                                }
                                                if remainder > 0 {
                                                    result.push_str(&pad_str[..remainder]);
                                                }
                                            }
                                            val = js_new_string(ctx, &result);
                                        } else {
                                            val = this_val;
                                        }
                                    } else {
                                        val = this_val;
                                    }
                                } else {
                                    val = this_val;
                                }
                            } else {
                                val = this_val;
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_parseInt__" {
                        if args.len() >= 1 {
                            if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                                if let Ok(s) = core::str::from_utf8(str_bytes) {
                                    if let Ok(n) = s.trim().parse::<i32>() {
                                        val = Value::from_int32(n);
                                    } else {
                                        val = number_to_value(ctx, f64::NAN);
                                    }
                                } else {
                                    val = number_to_value(ctx, f64::NAN);
                                }
                            } else if let Some(n) = args[0].int32() {
                                val = Value::from_int32(n);
                            } else {
                                val = number_to_value(ctx, f64::NAN);
                            }
                        } else {
                            val = number_to_value(ctx, f64::NAN);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_parseFloat__" {
                        if args.len() >= 1 {
                            if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                                if let Ok(s) = core::str::from_utf8(str_bytes) {
                                    if let Ok(n) = s.trim().parse::<f64>() {
                                        val = number_to_value(ctx, n);
                                    } else {
                                        val = number_to_value(ctx, f64::NAN);
                                    }
                                } else {
                                    val = number_to_value(ctx, f64::NAN);
                                }
                            } else if let Ok(n) = js_to_number(ctx, args[0]) {
                                val = number_to_value(ctx, n);
                            } else {
                                val = number_to_value(ctx, f64::NAN);
                            }
                        } else {
                            val = number_to_value(ctx, f64::NAN);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_isNaN__" {
                        if args.len() >= 1 {
                            if let Ok(n) = js_to_number(ctx, args[0]) {
                                val = Value::new_bool(n.is_nan());
                            } else {
                                val = Value::TRUE;
                            }
                        } else {
                            val = Value::TRUE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_isFinite__" {
                        if args.len() >= 1 {
                            if let Ok(n) = js_to_number(ctx, args[0]) {
                                val = Value::new_bool(n.is_finite());
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_pow__" {
                        if args.len() == 2 {
                            let base = js_to_number(ctx, args[0]).ok()?;
                            let exp = js_to_number(ctx, args[1]).ok()?;
                            val = number_to_value(ctx, base.powf(exp));
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_keys__" {
                        // Ported from mquickjs js_object_keys (mquickjs.c:13837)
                        // Simplified version using our existing object_keys() method
                        if args.len() == 1 {
                            let obj = args[0];
                            
                            // Get keys from the object
                            if let Some(keys) = ctx.object_keys(obj) {
                                // Create array for result
                                let arr = js_new_array(ctx, keys.len() as i32);
                                
                                // Populate array with key strings
                                for (i, key) in keys.iter().enumerate() {
                                    let key_str = js_new_string(ctx, key);
                                    js_set_property_uint32(ctx, arr, i as u32, key_str);
                                }
                                
                                val = arr;
                            } else {
                                // Not an object, return empty array
                                val = js_new_array(ctx, 0);
                            }
                        } else {
                            val = js_new_array(ctx, 0);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Array_isArray__" {
                        if args.len() == 1 {
                            if let Some(class_id) = ctx.object_class_id(args[0]) {
                                val = Value::new_bool(class_id == JSObjectClassEnum::Array as u32);
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Number_isInteger__" {
                        if args.len() == 1 {
                            if args[0].is_number() {
                                // It's an int32
                                val = Value::TRUE;
                            } else if let Ok(n) = js_to_number(ctx, args[0]) {
                                // Check if it's an integer value
                                val = Value::new_bool(n.is_finite() && n.fract() == 0.0);
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_String_fromCharCode__" {
                        if args.len() >= 1 {
                            let mut result = String::new();
                            for arg in args.iter() {
                                if let Some(code) = arg.int32() {
                                    if let Some(ch) = char::from_u32(code as u32) {
                                        result.push(ch);
                                    }
                                } else if let Ok(n) = js_to_number(ctx, *arg) {
                                    if let Some(ch) = char::from_u32(n as u32) {
                                        result.push(ch);
                                    }
                                }
                            }
                            val = js_new_string(ctx, &result);
                        } else {
                            val = js_new_string(ctx, "");
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Otherwise use the standard call mechanism
            for arg in args.iter().rev() {
                js_push_arg(ctx, *arg);
            }
            js_push_arg(ctx, val);
            js_push_arg(ctx, this_val);
            val = js_call(ctx, args.len() as i32);
            this_val = Value::UNDEFINED;
            rest = next;
            continue;
        }
        if rest_trim.starts_with('.') {
            let (name, next) = parse_identifier(&rest_trim[1..])?;
            this_val = val;
            
            // Handle special properties and methods
            // Array.length
            if name == "length" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        // Get array length through a public method
                        let len_val = js_get_property_str(ctx, val, "length");
                        if !len_val.is_undefined() {
                            val = len_val;
                            rest = next;
                            continue;
                        }
                    }
                }
                // String.length
                if let Some(bytes) = ctx.string_bytes(val) {
                    val = Value::from_int32(bytes.len() as i32);
                    rest = next;
                    continue;
                }
            }
            
            // Array.pop - create a callable wrapper
            if name == "pop" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        // Set val to a special marker that we'll detect in the call
                        val = js_new_string(ctx, "__builtin_array_pop__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // String.charAt - create a callable wrapper
            if name == "charAt" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_charAt__");
                    rest = next;
                    continue;
                }
            }
            
            // String.substring
            if name == "substring" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_substring__");
                    rest = next;
                    continue;
                }
            }
            
            // String.indexOf
            if name == "indexOf" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_indexOf__");
                    rest = next;
                    continue;
                }
            }
            
            // String.slice
            if name == "slice" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_slice__");
                    rest = next;
                    continue;
                }
            }
            
            // Array.shift
            if name == "shift" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_shift__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.unshift
            if name == "unshift" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_unshift__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.join
            if name == "join" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_join__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.toString
            if name == "toString" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_toString__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.reverse
            if name == "reverse" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_reverse__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // String.split
            if name == "split" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_split__");
                    rest = next;
                    continue;
                }
            }
            
            // String.toUpperCase
            if name == "toUpperCase" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_toUpperCase__");
                    rest = next;
                    continue;
                }
            }
            
            // String.toLowerCase
            if name == "toLowerCase" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_toLowerCase__");
                    rest = next;
                    continue;
                }
            }
            
            // String.trim
            if name == "trim" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_trim__");
                    rest = next;
                    continue;
                }
            }
            
            // String.startsWith
            if name == "startsWith" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_startsWith__");
                    rest = next;
                    continue;
                }
            }
            
            // String.endsWith
            if name == "endsWith" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_endsWith__");
                    rest = next;
                    continue;
                }
            }
            
            // String.includes
            if name == "includes" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_includes__");
                    rest = next;
                    continue;
                }
            }
            
            // String.repeat
            if name == "repeat" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_repeat__");
                    rest = next;
                    continue;
                }
            }
            
            // String.replace
            if name == "replace" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_replace__");
                    rest = next;
                    continue;
                }
            }
            
            // String.charCodeAt
            if name == "charCodeAt" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_charCodeAt__");
                    rest = next;
                    continue;
                }
            }
            
            // String.trimStart
            if name == "trimStart" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_trimStart__");
                    rest = next;
                    continue;
                }
            }
            
            // String.trimEnd
            if name == "trimEnd" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_trimEnd__");
                    rest = next;
                    continue;
                }
            }
            
            // String.padStart
            if name == "padStart" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_padStart__");
                    rest = next;
                    continue;
                }
            }
            
            // String.padEnd
            if name == "padEnd" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_padEnd__");
                    rest = next;
                    continue;
                }
            }
            
            // Array.concat
            if name == "concat" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_concat__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.lastIndexOf
            if name == "lastIndexOf" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_lastIndexOf__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.fill
            if name == "fill" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_fill__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.slice
            if name == "slice" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_slice__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.indexOf
            if name == "indexOf" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_indexOf__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Array.includes
            if name == "includes" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_includes__");
                        rest = next;
                        continue;
                    }
                }
            }
            
            // Math methods
            if let Some(bytes) = ctx.string_bytes(val) {
                if let Ok(marker) = core::str::from_utf8(bytes) {
                    if marker == "__builtin_Math__" {
                        match name {
                            "floor" => {
                                val = js_new_string(ctx, "__builtin_Math_floor__");
                                rest = next;
                                continue;
                            }
                            "ceil" => {
                                val = js_new_string(ctx, "__builtin_Math_ceil__");
                                rest = next;
                                continue;
                            }
                            "round" => {
                                val = js_new_string(ctx, "__builtin_Math_round__");
                                rest = next;
                                continue;
                            }
                            "abs" => {
                                val = js_new_string(ctx, "__builtin_Math_abs__");
                                rest = next;
                                continue;
                            }
                            "max" => {
                                val = js_new_string(ctx, "__builtin_Math_max__");
                                rest = next;
                                continue;
                            }
                            "min" => {
                                val = js_new_string(ctx, "__builtin_Math_min__");
                                rest = next;
                                continue;
                            }
                            "sqrt" => {
                                val = js_new_string(ctx, "__builtin_Math_sqrt__");
                                rest = next;
                                continue;
                            }
                            "pow" => {
                                val = js_new_string(ctx, "__builtin_Math_pow__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    } else if marker == "__builtin_Object__" {
                        match name {
                            "keys" => {
                                val = js_new_string(ctx, "__builtin_Object_keys__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    } else if marker == "__builtin_Array__" {
                        match name {
                            "isArray" => {
                                val = js_new_string(ctx, "__builtin_Array_isArray__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    } else if marker == "__builtin_Number__" {
                        match name {
                            "isInteger" => {
                                val = js_new_string(ctx, "__builtin_Number_isInteger__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    } else if marker == "__builtin_String__" {
                        match name {
                            "fromCharCode" => {
                                val = js_new_string(ctx, "__builtin_String_fromCharCode__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
            }
            
            val = js_get_property_str(ctx, val, name);
            rest = next;
            continue;
        }
        if rest_trim.starts_with('[') {
            let (inside, next) = extract_bracket(rest_trim)?;
            let idx_val = eval_expr(ctx, inside)?;
            if let Some(i) = idx_val.int32() {
                this_val = val;
                val = js_get_property_uint32(ctx, val, i as u32);
            } else if let Some(bytes) = ctx.string_bytes(idx_val) {
                let owned = bytes.to_vec();
                let name = core::str::from_utf8(&owned).ok()?;
                this_val = val;
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

/// Call a closure with arguments
fn call_closure(ctx: &mut JSContextImpl, func: JSValue, args: &[JSValue]) -> Option<JSValue> {
    // Get params and body from function object
    let params_val = js_get_property_str(ctx, func, "__params__");
    let body_val = js_get_property_str(ctx, func, "__body__");
    
    // Extract params string and make an owned copy
    let params_bytes = ctx.string_bytes(params_val)?;
    let params_str = core::str::from_utf8(params_bytes).ok()?.to_string();
    let param_names: Vec<String> = if params_str.is_empty() {
        Vec::new()
    } else {
        params_str.split(',').map(|s| s.trim().to_string()).collect()
    };
    
    // Extract body string and make an owned copy
    let body_bytes = ctx.string_bytes(body_val)?;
    let body_str = core::str::from_utf8(body_bytes).ok()?.to_string();
    
    // Get global object
    let saved_global = js_get_global_object(ctx);
    
    // Bind parameters to arguments in global scope
    for (i, param_name) in param_names.iter().enumerate() {
        let arg_val = args.get(i).copied().unwrap_or(Value::UNDEFINED);
        js_set_property_str(ctx, saved_global, param_name, arg_val);
    }
    
    // Execute the function body
    let result = eval_function_body(ctx, &body_str);
    
    // Clear return control after function completes
    if ctx.get_loop_control() == crate::context::LoopControl::Return {
        ctx.set_loop_control(crate::context::LoopControl::None);
    }
    
    // Clean up parameter bindings from global
    // Note: In a real implementation we'd restore the original values
    // For now, just leave them (simple implementation)
    
    result
}

/// Execute a function body and handle return statements
fn eval_function_body(ctx: &mut JSContextImpl, body: &str) -> Option<JSValue> {
    let stmts = split_statements(body)?;
    let mut last = Value::UNDEFINED;
    
    for stmt in stmts {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        
        // Check for break/continue
        if trimmed == "break" {
            ctx.set_loop_control(crate::context::LoopControl::Break);
            return Some(Value::UNDEFINED);
        }
        if trimmed == "continue" {
            ctx.set_loop_control(crate::context::LoopControl::Continue);
            return Some(Value::UNDEFINED);
        }
        
        // Check for return statement
        if trimmed.starts_with("return ") {
            let expr = &trimmed[7..]; // skip "return "
            if let Some(val) = eval_expr(ctx, expr.trim()) {
                ctx.set_return_value(val);
                ctx.set_loop_control(crate::context::LoopControl::Return);
                return Some(val);
            }
            return None;
        }
        if trimmed == "return" {
            ctx.set_return_value(Value::UNDEFINED);
            ctx.set_loop_control(crate::context::LoopControl::Return);
            return Some(Value::UNDEFINED);
        }
        
        // Check for if statement
        if trimmed.starts_with("if ") || trimmed.starts_with("if(") {
            last = parse_if_statement(ctx, trimmed)?;
            // Check if break/continue was set during statement execution
            if ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for while loop
        if trimmed.starts_with("while ") || trimmed.starts_with("while(") {
            last = parse_while_loop(ctx, trimmed)?;
            // Check if break/continue was set during statement execution
            if ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for for loop
        if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
            last = parse_for_loop(ctx, trimmed)?;
            // Check if break/continue was set during statement execution
            if ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Execute statement
        last = eval_expr(ctx, trimmed)?;
        
        // Check if break/continue was set during statement execution
        if ctx.get_loop_control() != crate::context::LoopControl::None {
            return Some(last);
        }
    }
    
    Some(last)
}

/// Parse function declaration: "function name(params) { body }"
/// Stores the function in the global object.
fn parse_function_declaration(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    if !s.starts_with("function ") {
        return None;
    }
    let rest = &s[9..]; // skip "function "
    
    // Parse function name
    let (name, after_name) = parse_identifier(rest)?;
    let after_name = after_name.trim_start();
    
    // Parse parameter list
    if !after_name.starts_with('(') {
        return None;
    }
    let (params_str, after_params) = extract_paren(after_name)?;
    let param_list = split_top_level(params_str)?;
    let mut params = Vec::new();
    for p in param_list {
        let p = p.trim();
        if !p.is_empty() {
            params.push(p.to_string());
        }
    }
    
    // Parse function body
    let after_params = after_params.trim_start();
    if !after_params.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_params)?;
    
    // Create a closure object
    let func = create_function(ctx, &params, body)?;
    
    // Store function in global object
    let global = js_get_global_object(ctx);
    js_set_property_str(ctx, global, name, func);
    
    Some(func)
}

/// Extract content within braces { }
fn extract_braces(s: &str) -> Option<(&str, &str)> {
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[1..i], &s[i + 1..]));
                }
            }
            b'"' | b'\'' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        break;
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Parse if statement: "if (condition) { block } else { block }"
fn parse_if_statement(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    let rest = if s.starts_with("if ") {
        &s[3..]
    } else if s.starts_with("if(") {
        &s[2..]
    } else {
        return None;
    };
    
    let rest = rest.trim_start();
    
    // Parse condition
    if !rest.starts_with('(') {
        return None;
    }
    let (condition, after_cond) = extract_paren(rest)?;
    let after_cond = after_cond.trim_start();
    
    // Parse then block
    if !after_cond.starts_with('{') {
        return None;
    }
    let (then_block, after_then) = extract_braces(after_cond)?;
    let after_then = after_then.trim_start();
    
    // Parse optional else block
    let else_block = if after_then.starts_with("else") {
        let after_else = after_then[4..].trim_start();
        if after_else.starts_with('{') {
            let (block, _) = extract_braces(after_else)?;
            Some(block)
        } else {
            None
        }
    } else {
        None
    };
    
    // Evaluate condition
    let cond_val = eval_expr(ctx, condition)?;
    let is_true = if cond_val.is_bool() {
        cond_val == Value::TRUE
    } else if let Some(n) = cond_val.int32() {
        n != 0
    } else {
        !cond_val.is_null() && !cond_val.is_undefined()
    };
    
    // Execute appropriate block
    if is_true {
        let result = eval_function_body(ctx, then_block)?;
        // Propagate return control
        if ctx.get_loop_control() == crate::context::LoopControl::Return {
            return Some(ctx.get_return_value());
        }
        Some(result)
    } else if let Some(else_body) = else_block {
        let result = eval_function_body(ctx, else_body)?;
        // Propagate return control
        if ctx.get_loop_control() == crate::context::LoopControl::Return {
            return Some(ctx.get_return_value());
        }
        Some(result)
    } else {
        Some(Value::UNDEFINED)
    }
}

/// Parse while loop: "while (condition) { block }"
fn parse_while_loop(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    let rest = if s.starts_with("while ") {
        &s[6..]
    } else if s.starts_with("while(") {
        &s[5..]
    } else {
        return None;
    };
    
    let rest = rest.trim_start();
    
    // Parse condition
    if !rest.starts_with('(') {
        return None;
    }
    let (condition, after_cond) = extract_paren(rest)?;
    let after_cond = after_cond.trim_start();
    
    // Parse body block
    if !after_cond.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_cond)?;
    
    // Execute loop
    let mut last = Value::UNDEFINED;
    loop {
        // Evaluate condition
        let cond_val = eval_expr(ctx, condition)?;
        let is_true = if cond_val.is_bool() {
            cond_val == Value::TRUE
        } else if let Some(n) = cond_val.int32() {
            n != 0
        } else {
            !cond_val.is_null() && !cond_val.is_undefined()
        };
        
        if !is_true {
            break;
        }
        
        // Execute body
        last = eval_function_body(ctx, body)?;
        
        // Check for loop control
        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                continue;
            }
            crate::context::LoopControl::Return => {
                // Propagate return up
                break;
            }
            crate::context::LoopControl::None => {}
        }
    }
    
    Some(last)
}

/// Parse for loop: "for (init; condition; update) { block }"
fn parse_for_loop(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    let rest = if s.starts_with("for ") {
        &s[4..]
    } else if s.starts_with("for(") {
        &s[3..]
    } else {
        return None;
    };
    
    let rest = rest.trim_start();
    
    // Parse for header
    if !rest.starts_with('(') {
        return None;
    }
    let (header, after_header) = extract_paren(rest)?;
    let after_header = after_header.trim_start();
    
    // Split header into init; condition; update
    let parts: Vec<&str> = header.split(';').collect();
    if parts.len() != 3 {
        return None;
    }
    let init = parts[0].trim();
    let condition = parts[1].trim();
    let update = parts[2].trim();
    
    // Parse body block
    if !after_header.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_header)?;
    
    // Execute loop
    // Initialize
    if !init.is_empty() {
        eval_expr(ctx, init)?;
    }
    
    let mut last = Value::UNDEFINED;
    loop {
        // Check condition
        if !condition.is_empty() {
            let cond_val = eval_expr(ctx, condition)?;
            let is_true = if cond_val.is_bool() {
                cond_val == Value::TRUE
            } else if let Some(n) = cond_val.int32() {
                n != 0
            } else {
                !cond_val.is_null() && !cond_val.is_undefined()
            };
            
            if !is_true {
                break;
            }
        }
        
        // Execute body
        last = eval_function_body(ctx, body)?;
        
        // Check for loop control
        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                // Continue to update
            }
            crate::context::LoopControl::Return => {
                // Propagate return up
                break;
            }
            crate::context::LoopControl::None => {}
        }
        
        // Execute update
        if !update.is_empty() {
            eval_expr(ctx, update)?;
        }
    }
    
    Some(last)
}

/// Create a function object with parameters and body
fn create_function(ctx: &mut JSContextImpl, params: &[String], body: &str) -> Option<JSValue> {
    // Encode params as a comma-separated string
    let params_str = params.join(",");
    let params_val = js_new_string(ctx, &params_str);
    
    // Encode body as a string
    let body_val = js_new_string(ctx, body);
    
    // Create closure object - use regular object for now
    let func = js_new_object(ctx);
    
    // Store params and body as properties
    js_set_property_str(ctx, func, "__params__", params_val);
    js_set_property_str(ctx, func, "__body__", body_val);
    js_set_property_str(ctx, func, "__closure__", Value::TRUE);
    
    Some(func)
}

fn eval_program(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let stmts = split_statements(src)?;
    let mut last = Value::UNDEFINED;
    let mut any = false;
    for stmt in stmts {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Check for break/continue
        if trimmed == "break" {
            ctx.set_loop_control(crate::context::LoopControl::Break);
            return Some(Value::UNDEFINED);
        }
        if trimmed == "continue" {
            ctx.set_loop_control(crate::context::LoopControl::Continue);
            return Some(Value::UNDEFINED);
        }
        // Check for function declaration
        if trimmed.starts_with("function ") {
            if let Some(val) = parse_function_declaration(ctx, trimmed) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for if statement
        if trimmed.starts_with("if ") || trimmed.starts_with("if(") {
            if let Some(val) = parse_if_statement(ctx, trimmed) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for while loop
        if trimmed.starts_with("while ") || trimmed.starts_with("while(") {
            if let Some(val) = parse_while_loop(ctx, trimmed) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for for loop
        if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
            if let Some(val) = parse_for_loop(ctx, trimmed) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        last = eval_expr(ctx, trimmed)?;
        any = true;
    }
    if any { Some(last) } else { None }
}

fn parse_json(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let mut parser = JsonParser::new(src.as_bytes());
    let val = parser.parse_value(ctx)?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return None;
    }
    Some(val)
}

struct JsonParser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\n' || b == b'\r' || b == b'\t' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn expect(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_value(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        self.skip_ws();
        match self.peek()? {
            b'{' => self.parse_object(ctx),
            b'[' => self.parse_array(ctx),
            b'"' => {
                let bytes = self.parse_string_bytes()?;
                let s = core::str::from_utf8(&bytes).ok()?;
                Some(js_new_string(ctx, s))
            }
            b'-' | b'0'..=b'9' => self.parse_number(ctx),
            b't' => {
                if self.consume_literal(b"true") {
                    Some(Value::TRUE)
                } else {
                    None
                }
            }
            b'f' => {
                if self.consume_literal(b"false") {
                    Some(Value::FALSE)
                } else {
                    None
                }
            }
            b'n' => {
                if self.consume_literal(b"null") {
                    Some(Value::NULL)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn consume_literal(&mut self, lit: &[u8]) -> bool {
        if self.input.len().saturating_sub(self.pos) < lit.len() {
            return false;
        }
        if &self.input[self.pos..self.pos + lit.len()] == lit {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn parse_array(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        if !self.expect(b'[') {
            return None;
        }
        self.skip_ws();
        let arr = js_new_array(ctx, 0);
        if arr.is_exception() {
            return None;
        }
        if self.expect(b']') {
            return Some(arr);
        }
        loop {
            let val = self.parse_value(ctx)?;
            let res = js_array_push(ctx, arr, val);
            if res.is_exception() {
                return None;
            }
            self.skip_ws();
            if self.expect(b',') {
                self.skip_ws();
                continue;
            }
            if self.expect(b']') {
                break;
            }
            return None;
        }
        Some(arr)
    }

    fn parse_object(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        if !self.expect(b'{') {
            return None;
        }
        self.skip_ws();
        let obj = js_new_object(ctx);
        if obj.is_exception() {
            return None;
        }
        if self.expect(b'}') {
            return Some(obj);
        }
        loop {
            self.skip_ws();
            let key_bytes = self.parse_string_bytes()?;
            let key = core::str::from_utf8(&key_bytes).ok()?;
            self.skip_ws();
            if !self.expect(b':') {
                return None;
            }
            let val = self.parse_value(ctx)?;
            let res = js_set_property_str(ctx, obj, key, val);
            if res.is_exception() {
                return None;
            }
            self.skip_ws();
            if self.expect(b',') {
                continue;
            }
            if self.expect(b'}') {
                break;
            }
            return None;
        }
        Some(obj)
    }

    fn parse_string_bytes(&mut self) -> Option<Vec<u8>> {
        if !self.expect(b'"') {
            return None;
        }
        let mut out = Vec::new();
        while let Some(b) = self.next() {
            match b {
                b'"' => return Some(out),
                b'\\' => {
                    let esc = self.next()?;
                    match esc {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0c),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let code = self.parse_hex4()? as u32;
                            let code = if is_high_surrogate(code) {
                                if self.next() != Some(b'\\') || self.next() != Some(b'u') {
                                    return None;
                                }
                                let low = self.parse_hex4()? as u32;
                                if !is_low_surrogate(low) {
                                    return None;
                                }
                                0x10000 + (((code - 0xD800) << 10) | (low - 0xDC00))
                            } else {
                                if is_low_surrogate(code) {
                                    return None;
                                }
                                code
                            };
                            let ch = char::from_u32(code)?;
                            let mut buf = [0u8; 4];
                            let s = ch.encode_utf8(&mut buf);
                            out.extend_from_slice(s.as_bytes());
                        }
                        _ => return None,
                    }
                }
                b if b < 0x20 => return None,
                _ => out.push(b),
            }
        }
        None
    }

    fn parse_hex4(&mut self) -> Option<u16> {
        let mut val = 0u16;
        for _ in 0..4 {
            let b = self.next()?;
            let digit = hex_val(b)? as u16;
            val = (val << 4) | digit;
        }
        Some(val)
    }

    fn parse_number(&mut self, ctx: &mut JSContextImpl) -> Option<JSValue> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek()? {
            b'0' => {
                self.pos += 1;
            }
            b'1'..=b'9' => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return None;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let s = core::str::from_utf8(&self.input[start..self.pos]).ok()?;
        let num = s.parse::<f64>().ok()?;
        Some(number_to_value(ctx, num))
    }
}

fn hex_val(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some((b - b'0') as u32),
        b'a'..=b'f' => Some((b - b'a' + 10) as u32),
        b'A'..=b'F' => Some((b - b'A' + 10) as u32),
        _ => None,
    }
}

fn is_high_surrogate(code: u32) -> bool {
    (0xD800..=0xDBFF).contains(&code)
}

fn is_low_surrogate(code: u32) -> bool {
    (0xDC00..=0xDFFF).contains(&code)
}

enum LValueKey {
    Name(String),
    Index(u32),
}

fn parse_lvalue(ctx: &mut JSContextImpl, src: &str) -> Option<(JSValue, LValueKey)> {
    let s = src.trim();
    let (base_str, tail) = split_base_and_tail(s)?;
    let mut base = if is_identifier(base_str) {
        let global = js_get_global_object(ctx);
        if tail.trim().is_empty() {
            return Some((global, LValueKey::Name(base_str.to_string())));
        }
        js_get_property_str(ctx, global, base_str)
    } else {
        eval_value(ctx, base_str)?
    };
    let mut rest = tail;
    loop {
        let rest_trim = rest.trim_start();
        if rest_trim.is_empty() {
            return None;
        }
        if rest_trim.starts_with('.') {
            let (name, next) = parse_identifier(&rest_trim[1..])?;
            if next.trim().is_empty() {
                return Some((base, LValueKey::Name(name.to_string())));
            }
            base = js_get_property_str(ctx, base, name);
            rest = next;
            continue;
        }
        if rest_trim.starts_with('[') {
            let (inside, next) = extract_bracket(rest_trim)?;
            let key_val = eval_expr(ctx, inside)?;
            let key = if let Some(i) = key_val.int32() {
                LValueKey::Index(i as u32)
            } else if let Some(bytes) = ctx.string_bytes(key_val) {
                let owned = bytes.to_vec();
                let name = core::str::from_utf8(&owned).ok()?.to_string();
                LValueKey::Name(name)
            } else {
                return None;
            };
            if next.trim().is_empty() {
                return Some((base, key));
            }
            match key {
                LValueKey::Index(idx) => {
                    base = js_get_property_uint32(ctx, base, idx);
                }
                LValueKey::Name(name) => {
                    base = js_get_property_str(ctx, base, &name);
                }
            }
            rest = next;
            continue;
        }
        return None;
    }
}

fn split_assignment(src: &str) -> Option<(&str, &str)> {
    let bytes = src.as_bytes();
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
        match b {
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = bytes.get(i.wrapping_sub(1)).copied().unwrap_or(b'\0');
                let next = bytes.get(i + 1).copied().unwrap_or(b'\0');
                // Skip if this is part of ==, ===, !=, !==, <=, or >=
                if prev == b'=' || next == b'=' || prev == b'!' || prev == b'<' || prev == b'>' {
                    continue;
                }
                let lhs = src[..i].trim();
                let rhs = src[i + 1..].trim();
                if lhs.is_empty() || rhs.is_empty() {
                    return None;
                }
                return Some((lhs, rhs));
            }
            _ => {}
        }
    }
    None
}

fn split_ternary(src: &str) -> Option<(&str, &str, &str)> {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut question_pos = None;
    
    // Find the ? at depth 0
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
        match b {
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => depth -= 1,
            b'?' if depth == 0 => {
                question_pos = Some(i);
                break;
            }
            _ => {}
        }
    }
    
    let q_pos = question_pos?;
    
    // Find the : at depth 0 after the ?
    depth = 0;
    in_string = false;
    for i in (q_pos + 1)..bytes.len() {
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
        match b {
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => depth -= 1,
            b':' if depth == 0 => {
                let cond = src[..q_pos].trim();
                let true_part = src[q_pos + 1..i].trim();
                let false_part = src[i + 1..].trim();
                if !cond.is_empty() && !true_part.is_empty() && !false_part.is_empty() {
                    return Some((cond, true_part, false_part));
                }
                return None;
            }
            _ => {}
        }
    }
    None
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
        if b == b'(' {
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
            b'{' => depth += 1,
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

fn extract_paren(src: &str) -> Option<(&str, &str)> {
    let bytes = src.as_bytes();
    if bytes.first().copied() != Some(b'(') {
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
        if b == b'(' {
            depth += 1;
            continue;
        }
        if b == b')' {
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

fn is_identifier(s: &str) -> bool {
    let (name, rest) = match parse_identifier(s) {
        Some(v) => v,
        None => return false,
    };
    !name.is_empty() && rest.trim().is_empty()
}

fn is_truthy(val: JSValue) -> bool {
    if val.is_bool() {
        val == Value::TRUE
    } else if val.is_number() {
        // Check if number is non-zero
        if let Some(n) = val.int32() {
            n != 0
        } else {
            // For float, treat as truthy unless it's exactly 0.0 or NaN
            // We can't easily check the exact value without context, so assume truthy
            true
        }
    } else {
        !val.is_null() && !val.is_undefined()
    }
}

fn call_c_function(
    _ctx: &mut JSContextImpl,
    func_idx: i32,
    params: JSValue,
    this_val: JSValue,
    args: &[JSValue],
) -> JSValue {
    let def = match _ctx.c_function_def(func_idx as usize) {
        Some(def) => *def,
        None => return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "unknown c function"),
    };
    match def.def_type {
        x if x == JSCFunctionDefEnum::Generic as u8 => {
            if let Some(f) = unsafe { def.func.generic } {
                return f(
                    _ctx as *mut JSContextImpl as *mut JSContext,
                    &this_val as *const JSValue as *mut JSValue,
                    args.len() as i32,
                    args.as_ptr() as *mut JSValue,
                );
            }
        }
        x if x == JSCFunctionDefEnum::GenericMagic as u8 => {
            if let Some(f) = unsafe { def.func.generic_magic } {
                return f(
                    _ctx as *mut JSContextImpl as *mut JSContext,
                    &this_val as *const JSValue as *mut JSValue,
                    args.len() as i32,
                    args.as_ptr() as *mut JSValue,
                    def.magic as i32,
                );
            }
        }
        x if x == JSCFunctionDefEnum::Constructor as u8 => {
            if let Some(f) = unsafe { def.func.constructor } {
                return f(
                    _ctx as *mut JSContextImpl as *mut JSContext,
                    &this_val as *const JSValue as *mut JSValue,
                    args.len() as i32,
                    args.as_ptr() as *mut JSValue,
                );
            }
        }
        x if x == JSCFunctionDefEnum::ConstructorMagic as u8 => {
            if let Some(f) = unsafe { def.func.constructor_magic } {
                return f(
                    _ctx as *mut JSContextImpl as *mut JSContext,
                    &this_val as *const JSValue as *mut JSValue,
                    args.len() as i32,
                    args.as_ptr() as *mut JSValue,
                    def.magic as i32,
                );
            }
        }
        x if x == JSCFunctionDefEnum::GenericParams as u8 => {
            if let Some(f) = unsafe { def.func.generic_params } {
                return f(
                    _ctx as *mut JSContextImpl as *mut JSContext,
                    &this_val as *const JSValue as *mut JSValue,
                    args.len() as i32,
                    args.as_ptr() as *mut JSValue,
                    params,
                );
            }
        }
        x if x == JSCFunctionDefEnum::FF as u8 => {
            if args.len() == 1 {
                if let Some(f) = unsafe { def.func.f_f } {
                    let v = js_to_number(_ctx, args[0]).unwrap_or(f64::NAN);
                    return js_new_float64(_ctx, f(v));
                }
            }
        }
        _ => {}
    }
    js_throw_error(_ctx, JSObjectClassEnum::TypeError, "unsupported c function")
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
        let val = eval_expr(ctx, value_src)?;
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

fn split_statements(src: &str) -> Option<Vec<&str>> {
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
            // After closing brace at depth 0, this could be end of a statement
            if depth == 0 && b == b'}' {
                // Check if there's an else clause following
                let rest = s[i + 1..].trim_start();
                if rest.starts_with("else ") || rest.starts_with("else{") {
                    // Don't split yet, continue to include else clause
                    continue;
                }
                // Check if the statement starts with control flow keyword
                let part = s[start..=i].trim();
                if part.starts_with("if ") || part.starts_with("if(")
                    || part.starts_with("while ") || part.starts_with("while(")
                    || part.starts_with("for ") || part.starts_with("for(")
                    || part.starts_with("function ") {
                    if !part.is_empty() {
                        out.push(part);
                    }
                    start = i + 1;
                }
            }
            continue;
        }
        // Split on semicolon or newline at depth 0
        if (b == b';' || b == b'\n') && depth == 0 {
            let part = s[start..i].trim();
            if !part.is_empty() {
                out.push(part);
            }
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

struct ArithParser<'a> {
    ctx: *mut JSContextImpl,
    input: &'a [u8],
    pos: usize,
}

impl<'a> ArithParser<'a> {
    fn new(ctx: &mut JSContextImpl, input: &'a [u8]) -> Self {
        Self { ctx, input, pos: 0 }
    }

    fn parse_expr(&mut self) -> Result<JSValue, ()> {
        self.parse_logical_or()
    }

    fn parse_logical_or(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_logical_and()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'|') && self.peek_at(1) == Some(b'|') {
                self.pos += 2;
                let rhs = self.parse_logical_and()?;
                value = self.logical_or(value, rhs)?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_logical_and(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_bitwise_or()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'&') && self.peek_at(1) == Some(b'&') {
                self.pos += 2;
                let rhs = self.parse_bitwise_or()?;
                value = self.logical_and(value, rhs)?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_bitwise_or(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_bitwise_xor()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'|') && self.peek_at(1) != Some(b'|') {
                self.pos += 1;
                let rhs = self.parse_bitwise_xor()?;
                value = self.bitwise_or(value, rhs)?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_bitwise_xor(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_bitwise_and()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'^') {
                self.pos += 1;
                let rhs = self.parse_bitwise_and()?;
                value = self.bitwise_xor(value, rhs)?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_bitwise_and(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_comparison()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'&') && self.peek_at(1) != Some(b'&') {
                self.pos += 1;
                let rhs = self.parse_comparison()?;
                value = self.bitwise_and(value, rhs)?;
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_comparison(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_shift()?;
        self.skip_ws();
        
        // Check for comparison operators
        let start_pos = self.pos;
        if let Some(first) = self.peek() {
            self.pos += 1;
            let op = match first {
                b'<' => {
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        &[b'<', b'='][..]
                    } else {
                        &[b'<'][..]
                    }
                }
                b'>' => {
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        &[b'>', b'='][..]
                    } else {
                        &[b'>'][..]
                    }
                }
                b'=' => {
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        if self.peek() == Some(b'=') {
                            self.pos += 1;
                            "===".as_bytes()
                        } else {
                            "==".as_bytes()
                        }
                    } else {
                        self.pos = start_pos;
                        return Ok(value);
                    }
                }
                b'!' => {
                    if self.peek() == Some(b'=') {
                        self.pos += 1;
                        if self.peek() == Some(b'=') {
                            self.pos += 1;
                            "!==".as_bytes()
                        } else {
                            "!=".as_bytes()
                        }
                    } else {
                        self.pos = start_pos;
                        return Ok(value);
                    }
                }
                _ => {
                    self.pos = start_pos;
                    return Ok(value);
                }
            };
            
            let rhs = self.parse_shift()?;
            value = self.compare_values(value, rhs, op)?;
        }
        Ok(value)
    }

    fn parse_shift(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_additive()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'<') && self.peek_at(1) == Some(b'<') {
                self.pos += 2;
                let rhs = self.parse_additive()?;
                value = self.left_shift(value, rhs)?;
            } else if self.peek() == Some(b'>') && self.peek_at(1) == Some(b'>') {
                self.pos += 2;
                if self.peek() == Some(b'>') {
                    self.pos += 1;
                    let rhs = self.parse_additive()?;
                    value = self.unsigned_right_shift(value, rhs)?;
                } else {
                    let rhs = self.parse_additive()?;
                    value = self.right_shift(value, rhs)?;
                }
            } else {
                break;
            }
        }
        Ok(value)
    }

    fn parse_additive(&mut self) -> Result<JSValue, ()> {
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
            value = if op == b'+' {
                self.add_values(value, rhs)?
            } else {
                self.sub_values(value, rhs)?
            };
        }
        Ok(value)
    }

    fn parse_term(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_exponent()?;
        loop {
            self.skip_ws();
            // Check for ** and skip it (handled by parse_exponent)
            if self.peek() == Some(b'*') && self.peek_at(1) == Some(b'*') {
                break;
            }
            let op = match self.peek() {
                Some(b'*') => b'*',
                Some(b'/') => b'/',
                Some(b'%') => b'%',
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_exponent()?;
            value = if op == b'*' {
                self.mul_values(value, rhs)?
            } else if op == b'/' {
                self.div_values(value, rhs)?
            } else {
                self.mod_values(value, rhs)?
            };
        }
        Ok(value)
    }

    fn parse_exponent(&mut self) -> Result<JSValue, ()> {
        let value = self.parse_unary()?;
        self.skip_ws();
        // Check for ** operator (right-associative)
        if self.peek() == Some(b'*') && self.peek_at(1) == Some(b'*') {
            self.pos += 2;
            let rhs = self.parse_exponent()?;  // Right-associative recursion
            self.pow_values(value, rhs)
        } else {
            Ok(value)
        }
    }

    fn parse_unary(&mut self) -> Result<JSValue, ()> {
        self.skip_ws();
        if let Some(b'+') = self.peek() {
            self.pos += 1;
            let val = self.parse_postfix()?;
            return self.unary_plus(val);
        }
        if let Some(b'-') = self.peek() {
            self.pos += 1;
            let val = self.parse_postfix()?;
            return self.unary_minus(val);
        }
        if let Some(b'!') = self.peek() {
            self.pos += 1;
            let val = self.parse_postfix()?;
            return self.logical_not(val);
        }
        if let Some(b'~') = self.peek() {
            self.pos += 1;
            let val = self.parse_postfix()?;
            return self.bitwise_not(val);
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_primary()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'.') => {
                    self.pos += 1;
                    let rest = core::str::from_utf8(&self.input[self.pos..]).map_err(|_| ())?;
                    let (prop, remaining) = parse_identifier(rest).ok_or(())?;
                    let consumed = rest.len() - remaining.len();
                    self.pos += consumed;
                    let ctx = unsafe { &mut *self.ctx };
                    value = js_get_property_str(ctx, value, prop);
                    if value.is_exception() {
                        return Err(());
                    }
                }
                Some(b'[') => {
                    self.pos += 1;
                    let index = self.parse_expr()?;
                    self.skip_ws();
                    if self.peek() != Some(b']') {
                        return Err(());
                    }
                    self.pos += 1;
                    let ctx = unsafe { &mut *self.ctx };
                    // Try as uint32 index first
                    if let Ok(idx) = js_to_uint32(ctx, index) {
                        value = js_get_property_uint32(ctx, value, idx);
                    } else if let Some(bytes) = ctx.string_bytes(index) {
                        let owned = bytes.to_vec();
                        if let Ok(s) = core::str::from_utf8(&owned) {
                            value = js_get_property_str(ctx, value, s);
                        } else {
                            return Err(());
                        }
                    } else {
                        return Err(());
                    }
                    if value.is_exception() {
                        return Err(());
                    }
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn parse_primary(&mut self) -> Result<JSValue, ()> {
        self.skip_ws();
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
        if self.peek() == Some(b'[') {
            return self.parse_array_literal();
        }
        if self.peek() == Some(b'{') {
            return self.parse_object_literal();
        }
        if self.peek() == Some(b'\"') || self.peek() == Some(b'\'') {
            return self.parse_string();
        }
        if matches!(self.peek(), Some(b'0'..=b'9') | Some(b'.')) {
            return self.parse_number_value();
        }
        self.parse_identifier_value()
    }

    fn parse_identifier_value(&mut self) -> Result<JSValue, ()> {
        let rest = core::str::from_utf8(&self.input[self.pos..]).map_err(|_| ())?;
        let (name, remaining) = parse_identifier(rest).ok_or(())?;
        let consumed = rest.len() - remaining.len();
        self.pos += consumed;
        match name {
            "true" => return Ok(Value::TRUE),
            "false" => return Ok(Value::FALSE),
            "null" => return Ok(Value::NULL),
            "undefined" => return Ok(Value::UNDEFINED),
            _ => {}
        }
        let ctx = unsafe { &mut *self.ctx };
        let global = js_get_global_object(ctx);
        let val = js_get_property_str(ctx, global, name);
        if val.is_exception() {
            return Err(());
        }
        Ok(val)
    }

    fn parse_string(&mut self) -> Result<JSValue, ()> {
        let quote = self.peek().ok_or(())?;
        self.pos += 1;
        let mut out = Vec::new();
        while let Some(b) = self.peek() {
            self.pos += 1;
            if b == quote {
                let s = core::str::from_utf8(&out).map_err(|_| ())?;
                let ctx = unsafe { &mut *self.ctx };
                return Ok(js_new_string(ctx, s));
            }
            if b == b'\\' {
                if let Some(esc) = self.peek() {
                    self.pos += 1;
                    out.push(esc);
                } else {
                    return Err(());
                }
            } else {
                out.push(b);
            }
        }
        Err(())
    }

    fn parse_number_value(&mut self) -> Result<JSValue, ()> {
        let num = self.parse_number_raw()?;
        let ctx = unsafe { &mut *self.ctx };
        let val = number_to_value(ctx, num);
        if val.is_exception() {
            return Err(());
        }
        Ok(val)
    }

    fn parse_number_raw(&mut self) -> Result<f64, ()> {
        self.skip_ws();
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
            }
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            Some(b'.') => {
                self.pos += 1;
            }
            _ => return Err(()),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(());
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(());
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let s = core::str::from_utf8(&self.input[start..self.pos]).map_err(|_| ())?;
        s.parse::<f64>().map_err(|_| ())
    }

    fn add_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let left_is_string = ctx.string_bytes(left).is_some();
        let right_is_string = ctx.string_bytes(right).is_some();
        let left_is_obj = ctx.object_class_id(left).is_some();
        let right_is_obj = ctx.object_class_id(right).is_some();
        if left_is_string || right_is_string || left_is_obj || right_is_obj {
            let ls = js_to_string(ctx, left);
            let rs = js_to_string(ctx, right);
            let lb = ctx.string_bytes(ls).ok_or(())?;
            let rb = ctx.string_bytes(rs).ok_or(())?;
            let mut out = Vec::with_capacity(lb.len() + rb.len());
            out.extend_from_slice(lb);
            out.extend_from_slice(rb);
            let val = js_new_string_len(ctx, &out);
            if val.is_exception() {
                return Err(());
            }
            return Ok(val);
        }
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln + rn);
        if val.is_exception() {
            Err(())
        } else {
            Ok(val)
        }
    }

    fn sub_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln - rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    fn mul_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln * rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    fn div_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln / rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    fn mod_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln % rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    fn pow_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln.powf(rn));
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    fn unary_plus(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let n = js_to_number(ctx, val).map_err(|_| ())?;
        let out = number_to_value(ctx, n);
        if out.is_exception() { Err(()) } else { Ok(out) }
    }

    fn unary_minus(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let n = js_to_number(ctx, val).map_err(|_| ())?;
        let out = number_to_value(ctx, -n);
        if out.is_exception() { Err(()) } else { Ok(out) }
    }

    fn compare_values(&mut self, left: JSValue, right: JSValue, op: &[u8]) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let result = if op.len() == 1 {
            match op[0] {
                b'<' => {
                    let ln = js_to_number(ctx, left).map_err(|_| ())?;
                    let rn = js_to_number(ctx, right).map_err(|_| ())?;
                    ln < rn
                }
                b'>' => {
                    let ln = js_to_number(ctx, left).map_err(|_| ())?;
                    let rn = js_to_number(ctx, right).map_err(|_| ())?;
                    ln > rn
                }
                _ => return Err(()),
            }
        } else if op.len() == 2 {
            match (op[0], op[1]) {
                (b'<', b'=') => {
                    let ln = js_to_number(ctx, left).map_err(|_| ())?;
                    let rn = js_to_number(ctx, right).map_err(|_| ())?;
                    ln <= rn
                }
                (b'>', b'=') => {
                    let ln = js_to_number(ctx, left).map_err(|_| ())?;
                    let rn = js_to_number(ctx, right).map_err(|_| ())?;
                    ln >= rn
                }
                (b'=', b'=') => {
                    // Simplified equality - in real JS, == does type coercion
                    if left.0 == right.0 {
                        true
                    } else {
                        let ln = js_to_number(ctx, left).ok();
                        let rn = js_to_number(ctx, right).ok();
                        if let (Some(l), Some(r)) = (ln, rn) {
                            l == r
                        } else {
                            false
                        }
                    }
                }
                (b'!', b'=') => {
                    // Simplified inequality
                    if left.0 == right.0 {
                        false
                    } else {
                        let ln = js_to_number(ctx, left).ok();
                        let rn = js_to_number(ctx, right).ok();
                        if let (Some(l), Some(r)) = (ln, rn) {
                            l != r
                        } else {
                            true
                        }
                    }
                }
                _ => return Err(()),
            }
        } else if op.len() == 3 {
            match (op[0], op[1], op[2]) {
                (b'=', b'=', b'=') => {
                    // Simplified equality
                    if left.0 == right.0 {
                        true
                    } else {
                        let ln = js_to_number(ctx, left).ok();
                        let rn = js_to_number(ctx, right).ok();
                        if let (Some(l), Some(r)) = (ln, rn) {
                            l == r
                        } else {
                            false
                        }
                    }
                }
                (b'!', b'=', b'=') => {
                    // Simplified inequality
                    if left.0 == right.0 {
                        false
                    } else {
                        let ln = js_to_number(ctx, left).ok();
                        let rn = js_to_number(ctx, right).ok();
                        if let (Some(l), Some(r)) = (ln, rn) {
                            l != r
                        } else {
                            true
                        }
                    }
                }
                _ => return Err(()),
            }
        } else {
            return Err(());
        };
        Ok(if result { Value::TRUE } else { Value::FALSE })
    }

    fn logical_and(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let left_truthy = self.is_truthy(left);
        if !left_truthy {
            Ok(left)
        } else {
            Ok(right)
        }
    }

    fn logical_or(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let left_truthy = self.is_truthy(left);
        if left_truthy {
            Ok(left)
        } else {
            Ok(right)
        }
    }

    fn logical_not(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let truthy = self.is_truthy(val);
        Ok(if truthy { Value::FALSE } else { Value::TRUE })
    }

    fn bitwise_and(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_int32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln & rn))
    }

    fn bitwise_or(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_int32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln | rn))
    }

    fn bitwise_xor(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_int32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln ^ rn))
    }

    fn bitwise_not(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let n = js_to_int32(ctx, val).map_err(|_| ())?;
        Ok(Value::from_int32(!n))
    }

    fn left_shift(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_uint32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln << (rn & 0x1f)))
    }

    fn right_shift(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_uint32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln >> (rn & 0x1f)))
    }

    fn unsigned_right_shift(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_uint32(ctx, left).map_err(|_| ())?;
        let rn = js_to_uint32(ctx, right).map_err(|_| ())?;
        let result = ln >> (rn & 0x1f);
        // Result needs to be treated as unsigned, so if it fits in i32 range, use that
        if result <= i32::MAX as u32 {
            Ok(Value::from_int32(result as i32))
        } else {
            Ok(number_to_value(ctx, result as f64))
        }
    }

    fn is_truthy(&self, val: JSValue) -> bool {
        if val.is_bool() {
            val == Value::TRUE
        } else if let Some(n) = val.int32() {
            n != 0
        } else if val.is_null() || val.is_undefined() {
            false
        } else {
            let ctx = unsafe { &*self.ctx };
            if let Some(f) = ctx.float_value(val) {
                f != 0.0 && !f.is_nan()
            } else {
                true // strings, objects, etc. are truthy
            }
        }
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.get(self.pos + offset).copied()
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

    fn parse_array_literal(&mut self) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        self.pos += 1; // skip '['
        self.skip_ws();
        
        let arr = js_new_array(ctx, 0);
        if arr.is_exception() {
            return Err(());
        }
        
        let mut idx = 0u32;
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.pos += 1;
                return Ok(arr);
            }
            
            if idx > 0 {
                if self.peek() != Some(b',') {
                    return Err(());
                }
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(arr);
                }
            }
            
            let elem = self.parse_expr()?;
            let res = js_set_property_uint32(ctx, arr, idx, elem);
            if res.is_exception() {
                return Err(());
            }
            idx += 1;
        }
    }

    fn parse_object_literal(&mut self) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        self.pos += 1; // skip '{'
        self.skip_ws();
        
        let obj = js_new_object(ctx);
        if obj.is_exception() {
            return Err(());
        }
        
        let mut first = true;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'}') {
                self.pos += 1;
                return Ok(obj);
            }
            
            if !first {
                if self.peek() != Some(b',') {
                    return Err(());
                }
                self.pos += 1;
                self.skip_ws();
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(obj);
                }
            }
            first = false;
            
            // Parse key (identifier or string)
            let key = if self.peek() == Some(b'\"') || self.peek() == Some(b'\'') {
                let key_val = self.parse_string()?;
                let bytes = ctx.string_bytes(key_val).ok_or(())?;
                let owned = bytes.to_vec();
                core::str::from_utf8(&owned).map_err(|_| ())?.to_string()
            } else {
                let rest = core::str::from_utf8(&self.input[self.pos..]).map_err(|_| ())?;
                let (name, remaining) = parse_identifier(rest).ok_or(())?;
                let consumed = rest.len() - remaining.len();
                self.pos += consumed;
                name.to_string()
            };
            
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(());
            }
            self.pos += 1;
            
            let value = self.parse_expr()?;
            let res = js_set_property_str(ctx, obj, &key, value);
            if res.is_exception() {
                return Err(());
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
}
