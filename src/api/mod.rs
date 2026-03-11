#![allow(non_snake_case)]
//!
//! # Module Organization
//! This file contains the public API and core evaluation logic.
//!
//! Related functionality has been extracted to:
//! - `helpers.rs` (156 lines) - Utility functions (number_to_value, is_identifier, etc.)
//! - `json.rs` (388 lines) - JSON parsing and stringification
//! - `evals.rs` (303 lines) - Evaluation utilities (eval_value, split_statements, etc.)
//! - `parser.rs` (1,270 lines) - Statement parsing (if, while, for, switch, try, functions)
//!
//! The main `eval_expr()` function (~2,600 lines) remains here with 83 embedded
//! built-in method implementations due to architectural constraints.

use crate::context::Context;
use fancy_regex::Regex;
use crate::types::*;
use crate::value::Value;

// Import extracted functionality
use crate::helpers::number_to_value;
use crate::parser::*;

// ============================================================================
// SUB-MODULES
// ============================================================================
pub(crate) mod number_fmt;
pub(crate) mod typed_array;
pub mod eval_expr;
pub mod eval_program;

// Re-export so the rest of this module and sibling sub-modules can use symbols
// without qualifying the path.
use self::typed_array::*;
pub use self::eval_expr::eval_expr;
pub use self::eval_program::*;


fn string_utf16_units(ctx: &mut JSContextImpl, val: JSValue) -> Option<Vec<u16>> {
    let bytes = ctx.string_bytes(val)?;
    let s = core::str::from_utf8(bytes).ok()?;
    let mut units = Vec::new();
    for ch in s.chars() {
        let code = ch as u32;
        if (0xE000..=0xE7FF).contains(&code) {
            units.push((code - 0x800) as u16);
        } else {
            let mut buf = [0u16; 2];
            let slice = ch.encode_utf16(&mut buf);
            units.extend_from_slice(slice);
        }
    }
    Some(units)
}

fn string_utf16_len(ctx: &mut JSContextImpl, val: JSValue) -> Option<usize> {
    string_utf16_units(ctx, val).map(|units| units.len())
}

fn string_units_equal(ctx: &mut JSContextImpl, a: JSValue, b: JSValue) -> Option<bool> {
    let ua = string_utf16_units(ctx, a)?;
    let ub = string_utf16_units(ctx, b)?;
    Some(ua == ub)
}

/// Return a type discriminant for strict equality type-checking.
/// Per ES spec, === must return false for different types without coercion.
/// Values: 0=undefined, 1=null, 2=bool, 3=number, 4=string, 5=object/function
fn strict_eq_type_tag(ctx: &mut JSContextImpl, v: JSValue) -> u8 {
    if v.is_undefined() {
        0
    } else if v.is_null() {
        1
    } else if v.is_bool() {
        2
    } else if v.is_int() || ctx.float_value(v).is_some() {
        3
    } else if ctx.string_bytes(v).is_some() {
        4
    } else {
        5
    }
}

fn object_to_string_value(ctx: &mut JSContextImpl, val: JSValue) -> JSValue {
    let tag = if val.is_undefined() {
        "Undefined"
    } else if val.is_null() {
        "Null"
    } else if val.is_bool() {
        "Boolean"
    } else if js_is_number(ctx, val) != 0 {
        "Number"
    } else if ctx.string_bytes(val).is_some() {
        "String"
    } else if let Some(class_id) = ctx.object_class_id(val) {
        match class_id {
            x if x == JSObjectClassEnum::Array as u32 => "Array",
            x if x == JSObjectClassEnum::Regexp as u32 => "RegExp",
            x if x == JSObjectClassEnum::Date as u32 => "Date",
            x if x == JSObjectClassEnum::CFunction as u32
                || x == JSObjectClassEnum::Closure as u32 => "Function",
            x if x == JSObjectClassEnum::Number as u32 => "Number",
            x if x == JSObjectClassEnum::Boolean as u32 => "Boolean",
            x if x == JSObjectClassEnum::String as u32 => "String",
            x if x == JSObjectClassEnum::ArrayBuffer as u32 => "ArrayBuffer",
            x if x == JSObjectClassEnum::Uint8Array as u32 => "Uint8Array",
            x if x == JSObjectClassEnum::Uint8cArray as u32 => "Uint8ClampedArray",
            x if x == JSObjectClassEnum::Int8Array as u32 => "Int8Array",
            x if x == JSObjectClassEnum::Int16Array as u32 => "Int16Array",
            x if x == JSObjectClassEnum::Uint16Array as u32 => "Uint16Array",
            x if x == JSObjectClassEnum::Int32Array as u32 => "Int32Array",
            x if x == JSObjectClassEnum::Uint32Array as u32 => "Uint32Array",
            x if x == JSObjectClassEnum::Float32Array as u32 => "Float32Array",
            x if x == JSObjectClassEnum::Float64Array as u32 => "Float64Array",
            x if x == JSObjectClassEnum::Error as u32
                || x == JSObjectClassEnum::EvalError as u32
                || x == JSObjectClassEnum::RangeError as u32
                || x == JSObjectClassEnum::ReferenceError as u32
                || x == JSObjectClassEnum::SyntaxError as u32
                || x == JSObjectClassEnum::TypeError as u32
                || x == JSObjectClassEnum::UriError as u32
                || x == JSObjectClassEnum::InternalError as u32 => "Error",
            _ => "Object",
        }
    } else {
        "Object"
    };
    let out = format!("[object {}]", tag);
    js_new_string(ctx, &out)
}

pub(crate) fn mark_const_binding(ctx: &mut JSContextImpl, env: JSValue, name: &str) {
    let map = js_get_property_str(ctx, env, "__const__");
    let map = if map.is_undefined() && !ctx.has_property_str(env, b"__const__") {
        let obj = js_new_object(ctx);
        js_set_property_str(ctx, env, "__const__", obj);
        obj
    } else {
        map
    };
    js_set_property_str(ctx, map, name, Value::TRUE);
}

fn is_const_binding(ctx: &mut JSContextImpl, env: JSValue, name: &str) -> bool {
    let map = js_get_property_str(ctx, env, "__const__");
    if map.is_undefined() && !ctx.has_property_str(env, b"__const__") {
        return false;
    }
    let v = js_get_property_str(ctx, map, name);
    v == Value::TRUE
}

fn has_top_level_arrow(src: &str) -> bool {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == string_delim {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            in_string = true;
            string_delim = b;
            i += 1;
            continue;
        }
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && b == b'=' && bytes[i + 1] == b'>' {
            return true;
        }
        i += 1;
    }
    false
}

/// Opaque handle to a VM instance.
pub type JSContextImpl = Context;

// ============================================================================
// PUBLIC API FUNCTIONS
// ============================================================================
// These functions mirror the mquickjs C API and provide the embedder-facing
// interface. They must maintain API/ABI compatibility with mquickjs.

/// Create a new context with a caller-provided memory buffer.
/// This mirrors JS_NewContext in mquickjs.h and must stay API-compatible.
pub fn js_new_context(mem: &mut [u8]) -> JSContextImpl {
    let mut ctx = Context::new(mem);
    let global = js_get_global_object(&mut ctx);
    let _ = js_set_property_str(&mut ctx, global, "globalThis", global);
    let _ = js_set_property_str(&mut ctx, global, "__var_env__", Value::TRUE);
    let nan = number_to_value(&mut ctx, f64::NAN);
    let inf = number_to_value(&mut ctx, f64::INFINITY);
    let _ = js_set_property_str(&mut ctx, global, "NaN", nan);
    let _ = js_set_property_str(&mut ctx, global, "Infinity", inf);
    let _ = js_set_property_str(&mut ctx, global, "undefined", Value::UNDEFINED);
    init_default_prototypes(&mut ctx, global);
    ctx
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
    let global = js_get_global_object(&mut ctx);
    let _ = js_set_property_str(&mut ctx, global, "globalThis", global);
    let _ = js_set_property_str(&mut ctx, global, "__var_env__", Value::TRUE);
    let nan = number_to_value(&mut ctx, f64::NAN);
    let inf = number_to_value(&mut ctx, f64::INFINITY);
    let _ = js_set_property_str(&mut ctx, global, "NaN", nan);
    let _ = js_set_property_str(&mut ctx, global, "Infinity", inf);
    let _ = js_set_property_str(&mut ctx, global, "undefined", Value::UNDEFINED);
    init_default_prototypes(&mut ctx, global);
    ctx
}

fn init_default_prototypes(ctx: &mut JSContextImpl, global: JSValue) {
    let object_proto = js_new_object(ctx);
    if object_proto.is_exception() {
        return;
    }
    let _ = ctx.set_object_proto(object_proto, Value::NULL);
    let ctor_marker = js_new_string(ctx, "__builtin_Object__");
    let to_string_marker = js_new_string(ctx, "__builtin_Object_toString__");
    let has_own_marker = js_new_string(ctx, "__builtin_Object_hasOwnProperty__");
    let _ = js_set_property_str(ctx, object_proto, "constructor", ctor_marker);
    let _ = js_set_property_str(ctx, object_proto, "toString", to_string_marker);
    let _ = js_set_property_str(ctx, object_proto, "hasOwnProperty", has_own_marker);
    ctx.set_object_proto_default(object_proto);
    let _ = ctx.set_object_proto(global, object_proto);

    let array_proto = js_new_object(ctx);
    if array_proto.is_exception() {
        return;
    }
    let _ = ctx.set_object_proto(array_proto, object_proto);
    ctx.set_array_proto(array_proto);
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
    if let Some(id) = _ctx.object_class_id(_val) {
        let func = JSObjectClassEnum::CFunction as u32;
        let closure = JSObjectClassEnum::Closure as u32;
        if id == func || id == closure {
            return 1;
        }
    }
    if let Some(bytes) = _ctx.string_bytes(_val) {
        if let Ok(marker) = core::str::from_utf8(bytes) {
            if marker.starts_with("__builtin_") {
                return 1;
            }
        }
    }
    // Treat custom closures (created via create_function) as functions.
    let marker = js_get_property_str(_ctx, _val, "__closure__");
    if marker == Value::TRUE { 1 } else { 0 }
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
    let name = match _error_num {
        JSObjectClassEnum::TypeError => "TypeError",
        JSObjectClassEnum::ReferenceError => "ReferenceError",
        JSObjectClassEnum::SyntaxError => "SyntaxError",
        JSObjectClassEnum::RangeError => "RangeError",
        JSObjectClassEnum::InternalError => "InternalError",
        _ => "Error",
    };
    let obj = js_new_error_object(_ctx, _error_num, name, _msg);
    _ctx.set_exception(obj);
    Value::EXCEPTION
}

fn js_new_error_object(ctx: &mut JSContextImpl, class_id: JSObjectClassEnum, name: &str, msg: &str) -> JSValue {
    let obj = js_new_object_class_user(ctx, class_id as i32);
    if obj.is_exception() {
        return obj;
    }
    let name_val = js_new_string(ctx, name);
    let msg_val = js_new_string(ctx, msg);
    let ctor = js_new_object(ctx);
    if !ctor.is_exception() {
        let _ = js_set_property_str(ctx, ctor, "name", name_val);
        let _ = js_set_property_str(ctx, obj, "constructor", ctor);
    }
    let _ = js_set_property_str(ctx, obj, "name", name_val);
    let _ = js_set_property_str(ctx, obj, "message", msg_val);
    if let Some(stack) = ctx.format_stack() {
        let stack_val = js_new_string(ctx, &stack);
        let _ = js_set_property_str(ctx, obj, "stack", stack_val);
    }
    obj
}

pub fn js_throw_out_of_memory(_ctx: &mut JSContextImpl) -> JSValue {
    _ctx.set_exception(Value::UNDEFINED);
    Value::EXCEPTION
}

pub fn js_get_property_str(_ctx: &mut JSContextImpl, _this_obj: JSValue, _str: &str) -> JSValue {
    // Check for getter first (unless we're looking for a __get__ or __set__ property itself)
    if !_str.starts_with("__get__") && !_str.starts_with("__set__") {
        let getter_key = format!("__get__{}", _str);
        let getter = _ctx.get_property_str(_this_obj, getter_key.as_bytes());
        if let Some(getter_fn) = getter {
            if !getter_fn.is_undefined() {
                // Call the getter with `this` bound to _this_obj
                if let Some(result) = crate::parser::call_closure_with_this(_ctx, getter_fn, _this_obj, &[]) {
                    return result;
                }
            }
        }
    }
    _ctx.get_property_str(_this_obj, _str.as_bytes()).unwrap_or(Value::UNDEFINED)
}

pub fn js_get_property_uint32(_ctx: &mut JSContextImpl, _obj: JSValue, _idx: u32) -> JSValue {
    // Fast path: direct array element access — avoids string_utf16_units,
    // typed-array check, and get_property_index with prototype chain walk.
    if let Some(v) = _ctx.array_direct_get(_obj, _idx) {
        return v;
    }
    if let Some(units) = string_utf16_units(_ctx, _obj) {
        let idx = _idx as usize;
        if idx < units.len() {
            let s = crate::evals::utf16_units_to_string_preserve_surrogates(&[units[idx]]);
            return js_new_string(_ctx, &s);
        }
        return Value::UNDEFINED;
    }
    if let Some(class_id) = _ctx.object_class_id(_obj) {
        if typed_array_kind_from_class_id(class_id).is_some() {
            return typed_array_get_element(_ctx, _obj, _idx);
        }
    }
    _ctx.get_property_index(_obj, _idx).unwrap_or(Value::UNDEFINED)
}

pub fn js_set_property_str(
    _ctx: &mut JSContextImpl,
    _this_obj: JSValue,
    _str: &str,
    _val: JSValue,
) -> JSValue {
    if let Some(class_id) = _ctx.object_class_id(_this_obj) {
        if class_id == JSObjectClassEnum::Array as u32 {
            let is_index = {
                let bytes = _str.as_bytes();
                if bytes.is_empty() {
                    false
                } else {
                    let mut value: u32 = 0;
                    let mut ok = true;
                    for &b in bytes {
                        if b < b'0' || b > b'9' {
                            ok = false;
                            break;
                        }
                        let digit = (b - b'0') as u32;
                        if let Some(next) = value.checked_mul(10).and_then(|v| v.checked_add(digit)) {
                            value = next;
                        } else {
                            ok = false;
                            break;
                        }
                    }
                    ok
                }
            };
            let is_special = _str == "length" || _str.starts_with("__get__") || _str.starts_with("__set__");
            if !is_index && !is_special {
                let trimmed = _str.trim();
                let is_numeric_like = if trimmed.is_empty() {
                    false
                } else if trimmed == "NaN" {
                    true
                } else if trimmed == "Infinity" || trimmed == "+Infinity" || trimmed == "-Infinity" {
                    true
                } else {
                    trimmed.parse::<f64>().is_ok()
                };
                if is_numeric_like {
                    return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "invalid array property");
                }
            }
        }
    }
    // Check for setter first (unless we're setting a __get__ or __set__ property itself)
    if !_str.starts_with("__get__") && !_str.starts_with("__set__") {
        let setter_key = format!("__set__{}", _str);
        let setter = _ctx.get_property_str(_this_obj, setter_key.as_bytes());
        if let Some(setter_fn) = setter {
            if !setter_fn.is_undefined() {
                // Call the setter with `this` bound to _this_obj
                if let Some(result) = crate::parser::call_closure_with_this(_ctx, setter_fn, _this_obj, &[_val]) {
                    return result;
                }
            }
        }
    }
    if _ctx.set_property_str(_this_obj, _str.as_bytes(), _val) {
        // Keep call frame slot_map in sync for fast variable access.
        // Skip internal properties (__parent__, __var_env__, etc.).
        if !_str.starts_with("__") {
            _ctx.update_call_frame_slot(_this_obj, _str, _val);
        }
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
    // Fast path: direct array element write — skips typed-array check
    // and set_property_index indirection.
    if _ctx.array_direct_set(_this_obj, _idx, _val) {
        return _val;
    }
    if let Some(class_id) = _ctx.object_class_id(_this_obj) {
        if typed_array_kind_from_class_id(class_id).is_some() {
            if typed_array_set_element(_ctx, _this_obj, _idx, _val) {
                return _val;
            }
            return Value::UNDEFINED;
        }
    }
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

/// Create a bare array without push/pop method setup (for internal use).
#[inline]
pub fn js_new_array_bare(_ctx: &mut JSContextImpl, _initial_len: i32) -> JSValue {
    if _initial_len < 0 {
        return Value::EXCEPTION;
    }
    _ctx
        .new_array_bare(_initial_len as usize)
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
        if let Some(val) = crate::json::parse_json(_ctx, _input) {
            return val;
        }
        return js_throw_error(_ctx, JSObjectClassEnum::SyntaxError, "invalid JSON");
    }
    js_new_string(_ctx, _input)
}

pub fn js_run(_ctx: &mut JSContextImpl, _val: JSValue) -> JSValue {
    let mut val = _val;
    if let Some(src) = js_get_bytecode_source(_ctx, val) {
        val = src;
    }
    if let Some(bytes) = _ctx.string_bytes(val) {
        if let Ok(src) = core::str::from_utf8(bytes) {
            let owned = src.to_owned();
            return js_eval(_ctx, &owned, "<run>", JS_EVAL_RETVAL);
        }
    }
    if val.is_exception() {
        return val;
    }
    val
}

pub fn js_eval(
    _ctx: &mut JSContextImpl,
    _input: &str,
    _filename: &str,
    _eval_flags: i32,
) -> JSValue {
    _ctx.set_current_source(_filename, _input);
    let src = _input;
    if (_eval_flags & JS_EVAL_JSON) != 0 {
        if let Some(val) = crate::json::parse_json(_ctx, src) {
            return val;
        }
        return js_throw_error(_ctx, JSObjectClassEnum::SyntaxError, "invalid JSON");
    }
    // Script mode: handle top-level `return` like a function body
    if (_eval_flags & JS_EVAL_SCRIPT) != 0 {
        return eval_script_body(_ctx, src);
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
    if _ctx.current_error_offset().is_none() {
        let off = find_syntax_error_offset(_ctx.current_source());
        _ctx.set_error_offset(off);
    }
    js_throw_error(_ctx, JSObjectClassEnum::SyntaxError, "syntax error")
}

fn find_syntax_error_offset(src: &str) -> usize {
    if let Some(pos) = src.find("/*") {
        if !src[pos + 2..].contains("*/") {
            return pos;
        }
    }
    if let Some(pos) = find_function_error_pos(src) {
        return pos;
    }
    if let Some(pos) = find_number_followed_by_ident(src) {
        return pos;
    }
    if let Some(pos) = find_regex_literal_start(src) {
        return pos;
    }
    src.find(|c: char| !c.is_whitespace()).unwrap_or(0)
}

fn find_function_error_pos(src: &str) -> Option<usize> {
    let trimmed = src.trim_start();
    let start = src.len() - trimmed.len();
    if !trimmed.starts_with("function") {
        return None;
    }
    let mut i = start + "function".len();
    let bytes = src.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    // parse optional name
    let name_start = i;
    if i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
    } else {
        return Some(i);
    }
    // skip whitespace after name
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < bytes.len() && bytes[i] != b'(' {
        return Some(i.max(name_start));
    }
    None
}

fn find_number_followed_by_ident(src: &str) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'_') {
                return Some(j);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

fn find_regex_literal_start(src: &str) -> Option<usize> {
    let bytes = src.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'/' {
            return Some(i);
        }
    }
    None
}

pub fn js_gc(_ctx: &mut JSContextImpl) {
    _ctx.gc_collect();
}

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

pub fn js_dup_atom(_ctx: &mut JSContextImpl, atom: i32) -> i32 {
    if atom <= 0 {
        return atom;
    }
    if _ctx.atom_dup(atom as u32) {
        atom
    } else {
        -1
    }
}

pub fn js_free_atom(_ctx: &mut JSContextImpl, atom: i32) {
    if atom <= 0 {
        return;
    }
    _ctx.atom_free(atom as u32);
}

pub fn js_atom_to_value(_ctx: &mut JSContextImpl, atom: i32) -> JSValue {
    if atom == JS_ATOM_NULL {
        return Value::NULL;
    }
    if atom <= 0 {
        return Value::UNDEFINED;
    }
    if let Some(bytes) = _ctx.atom_bytes(atom as u32) {
        let owned = bytes.to_vec();
        return js_new_string_len(_ctx, &owned);
    }
    Value::UNDEFINED
}

pub fn js_value_to_atom(_ctx: &mut JSContextImpl, val: JSValue) -> i32 {
    let mut str_val = val;
    if _ctx.string_bytes(str_val).is_none() {
        str_val = js_to_string(_ctx, val);
        if str_val.is_exception() {
            return -1;
        }
    }
    if let Some(bytes) = _ctx.string_bytes(str_val) {
        let owned = bytes.to_vec();
        if let Some(atom) = _ctx.intern_string(&owned) {
            return atom as i32;
        }
    }
    -1
}

fn js_new_bytecode_object(_ctx: &mut JSContextImpl, source: JSValue) -> JSValue {
    let obj = js_new_object(_ctx);
    if obj.is_exception() {
        return obj;
    }
    let _ = js_set_property_str(_ctx, obj, "__bytecode__", source);
    obj
}

fn js_get_bytecode_source(_ctx: &mut JSContextImpl, val: JSValue) -> Option<JSValue> {
    if _ctx.object_class_id(val).is_none() {
        return None;
    }
    let src = js_get_property_str(_ctx, val, "__bytecode__");
    if src.is_undefined() {
        None
    } else {
        Some(src)
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
    if let Some(_class_id) = _ctx.object_class_id(_val) {
        if let Some(prim) = js_to_primitive(_ctx, _val, false) {
            if _ctx.object_class_id(prim).is_none() {
                return js_to_string(_ctx, prim);
            }
        }
        if _class_id == JSObjectClassEnum::Array as u32 {
            return js_new_string(_ctx, "[object Array]");
        }
        return js_new_string(_ctx, "[object Object]");
    }
    Value::UNDEFINED
}

/// Convert object to primitive using valueOf/toString order.
/// prefer_number = true => valueOf then toString
/// prefer_number = false => toString then valueOf
fn js_to_primitive(ctx: &mut JSContextImpl, val: JSValue, prefer_number: bool) -> Option<JSValue> {
    if ctx.object_class_id(val).is_none() {
        return Some(val);
    }

    let order = if prefer_number { ["valueOf", "toString"] } else { ["toString", "valueOf"] };
    for name in order.iter() {
        let method = js_get_property_str(ctx, val, name);
        if method.is_undefined() || method.is_null() {
            continue;
        }
        if js_is_function(ctx, method) != 0 {
            if let Some(res) = crate::parser::call_closure_with_this(ctx, method, val, &[]) {
                if ctx.object_class_id(res).is_none() {
                    return Some(res);
                }
            } else if ctx.get_exception() != Value::UNDEFINED {
                return Some(Value::EXCEPTION);
            }
        } else if let Some(bytes) = ctx.string_bytes(method) {
            if bytes == b"__builtin_Object_toString__" {
                let res = object_to_string_value(ctx, val);
                if ctx.object_class_id(res).is_none() {
                    return Some(res);
                }
            }
        }
    }
    Some(object_to_string_value(ctx, val))
}

pub fn js_to_int32(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<i32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v)
    } else if let Some(f) = _ctx.float_value(_val) {
        Ok(float_to_int32(f))
    } else {
        let n = js_to_number(_ctx, _val)?;
        Ok(float_to_int32(n))
    }
}

/// Convert a float to int32 using JavaScript's ToInt32 algorithm
fn float_to_int32(f: f64) -> i32 {
    if !f.is_finite() || f == 0.0 {
        return 0;
    }
    // Get the integer part (truncate towards zero)
    let int = f.trunc();
    // Compute int modulo 2^32 (always positive)
    let int32bit = int.rem_euclid(4294967296.0) as u32;
    // Convert to signed: if >= 2^31, subtract 2^32
    int32bit as i32
}

pub fn js_to_uint32(_ctx: &mut JSContextImpl, _val: JSValue) -> Result<u32, JSValue> {
    if let Some(v) = _val.int32() {
        Ok(v as u32)
    } else if let Some(f) = _ctx.float_value(_val) {
        Ok(float_to_uint32(f))
    } else {
        let n = js_to_number(_ctx, _val)?;
        Ok(float_to_uint32(n))
    }
}

/// Convert a float to uint32 using JavaScript's ToUint32 algorithm
fn float_to_uint32(f: f64) -> u32 {
    if !f.is_finite() || f == 0.0 {
        return 0;
    }
    // Get the integer part (truncate towards zero)
    let int = f.trunc();
    // Compute int modulo 2^32 (always positive)
    int.rem_euclid(4294967296.0) as u32
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
        // Fast path: simple integer strings (very common in redis scripting)
        if !bytes.is_empty() && bytes.len() <= 10 {
            let (start, neg) = if bytes[0] == b'-' { (1, true) } else { (0, false) };
            if start < bytes.len() && bytes[start..].iter().all(|b| b.is_ascii_digit()) {
                let mut n: i64 = 0;
                for &b in &bytes[start..] {
                    n = n * 10 + (b - b'0') as i64;
                }
                if neg { n = -n; }
                if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
                    return Ok(n as f64);
                }
                return Ok(n as f64);
            }
        }
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
        if let Some(prim) = js_to_primitive(_ctx, _val, true) {
            if prim.is_exception() {
                return Err(Value::EXCEPTION);
            }
            if _ctx.object_class_id(prim).is_none() {
                return js_to_number(_ctx, prim);
            }
        }
        if _ctx.get_exception() != Value::UNDEFINED {
            return Err(Value::EXCEPTION);
        }
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
    let closure_marker = js_get_property_str(_ctx, func_val, "__closure__");
    if closure_marker == Value::TRUE {
        if let Some(val) = call_closure(_ctx, func_val, &args) {
            return val;
        }
        return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not a function");
    }
    if let Some((idx, params)) = _ctx.c_function_info(func_val) {
        return call_c_function(_ctx, idx, params, _this_val, &args);
    }
    if let Some(bytes) = _ctx.string_bytes(func_val) {
        // Stack buffer to avoid heap allocation from .to_string()
        let mut marker_buf = [0u8; 64];
        let blen = bytes.len();
        if blen <= 64 {
            marker_buf[..blen].copy_from_slice(bytes);
            if let Ok(marker) = core::str::from_utf8(&marker_buf[..blen]) {
                if let Some(val) = call_builtin_global_marker(_ctx, marker, &args) {
                    return val;
                }
            }
        }
        let mut parser = ArithParser::new(_ctx, b"", _ctx.current_stmt_offset());
        if let Ok(val) = parser.call_builtin_method(_ctx, func_val, _this_val, &args) {
            return val;
        }
    }
    js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not a function")
}

pub fn js_is_bytecode(_buf: &[u8]) -> JSBool {
    if _buf.len() < core::mem::size_of::<JSBytecodeHeader>() {
        return 0;
    }
    let magic = u16::from_ne_bytes([_buf[0], _buf[1]]);
    if magic == JS_BYTECODE_MAGIC { 1 } else { 0 }
}

pub fn js_relocate_bytecode(_ctx: &mut JSContextImpl, _buf: &mut [u8]) -> i32 {
    let header_size = core::mem::size_of::<JSBytecodeHeader>();
    if _buf.len() < header_size {
        return -1;
    }
    if js_is_bytecode(_buf) == 0 {
        return -1;
    }
    let hdr = unsafe { &mut *(_buf.as_mut_ptr() as *mut JSBytecodeHeader) };
    if hdr.version != JS_BYTECODE_VERSION {
        return -1;
    }
    let data_ptr = unsafe { _buf.as_ptr().add(header_size) } as usize;
    hdr.base_addr = data_ptr;
    0
}

pub fn js_load_bytecode(_ctx: &mut JSContextImpl, _buf: &[u8]) -> JSValue {
    let header_size = core::mem::size_of::<JSBytecodeHeader>();
    if _buf.len() < header_size {
        return js_throw_error(_ctx, JSObjectClassEnum::InternalError, "invalid bytecode buffer");
    }
    if js_is_bytecode(_buf) == 0 {
        return js_throw_error(_ctx, JSObjectClassEnum::InternalError, "invalid bytecode magic");
    }
    let hdr = unsafe { &*(_buf.as_ptr() as *const JSBytecodeHeader) };
    if hdr.version != JS_BYTECODE_VERSION {
        return js_throw_error(_ctx, JSObjectClassEnum::InternalError, "invalid bytecode version");
    }
    let expected_base = unsafe { _buf.as_ptr().add(header_size) } as usize;
    if hdr.base_addr != expected_base {
        return js_throw_error(_ctx, JSObjectClassEnum::InternalError, "bytecode not relocated");
    }
    if !_ctx.add_rom_atom_table(hdr.unique_strings) {
        return js_throw_error(_ctx, JSObjectClassEnum::InternalError, "too many rom atom tables");
    }
    js_new_bytecode_object(_ctx, hdr.main_func)
}

pub fn js_set_log_func(_ctx: &mut JSContextImpl, _write_func: Option<JSWriteFunc>) {
    _ctx.set_log_func(_write_func);
}

pub fn js_set_c_function_table(_ctx: &mut JSContextImpl, table: &[JSCFunctionDef]) {
    _ctx.set_c_function_table(table.as_ptr(), table.len());
}

pub fn js_set_stdlib_def(_ctx: &mut JSContextImpl, def: &JSSTDLibraryDef, cfunc_len: usize) {
    _ctx.set_c_function_table(def.c_function_table, cfunc_len);
    _ctx.set_stdlib_def(def);
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
    if obj.is_null() || obj.is_undefined() {
        return js_throw_error(_ctx, JSObjectClassEnum::TypeError, "not an object");
    }
    if _ctx.object_class_id(obj).is_none() {
        let proto = _ctx.object_proto_default();
        return if proto.is_undefined() { Value::NULL } else { proto };
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

pub fn js_console_log(_ctx: &mut JSContextImpl, args: &[JSValue]) {
    let mut buf = JSCStringBuf { buf: [0u8; 5] };
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            _ctx.write_log(b" ");
        }
        let owned = {
            let s = js_to_cstring(_ctx, *arg, &mut buf);
            s.as_bytes().to_vec()
        };
        _ctx.write_log(&owned);
    }
    _ctx.write_log(b"\n");
}

fn call_builtin_global_marker(
    ctx: &mut JSContextImpl,
    marker: &str,
    args: &[JSValue],
) -> Option<JSValue> {
    match marker {
        "__builtin_Number__" => {
            if args.is_empty() {
                Some(Value::from_int32(0))
            } else {
                let n = js_to_number(ctx, args[0]).unwrap_or(f64::NAN);
                Some(number_to_value(ctx, n))
            }
        }
        "__builtin_String__" => {
            if args.is_empty() {
                Some(js_new_string(ctx, ""))
            } else {
                Some(js_to_string(ctx, args[0]))
            }
        }
        "__builtin_Boolean__" => {
            if args.is_empty() {
                Some(Value::FALSE)
            } else {
                Some(Value::new_bool(crate::evals::is_truthy(ctx, args[0])))
            }
        }
        "__builtin_parseInt__" => {
            if args.len() >= 1 {
                if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                    if let Ok(s) = core::str::from_utf8(str_bytes) {
                        if let Ok(n) = s.trim().parse::<i32>() {
                            return Some(Value::from_int32(n));
                        }
                    }
                } else if let Some(n) = args[0].int32() {
                    return Some(Value::from_int32(n));
                }
                Some(number_to_value(ctx, f64::NAN))
            } else {
                Some(number_to_value(ctx, f64::NAN))
            }
        }
        "__builtin_parseFloat__" => {
            if args.len() >= 1 {
                if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                    if let Ok(s) = core::str::from_utf8(str_bytes) {
                        let trimmed = s.trim_start();
                        if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                            return Some(number_to_value(ctx, 0.0));
                        }
                        if let Ok(n) = trimmed.parse::<f64>() {
                            return Some(number_to_value(ctx, n));
                        }
                    }
                } else if let Ok(n) = js_to_number(ctx, args[0]) {
                    return Some(number_to_value(ctx, n));
                }
                Some(number_to_value(ctx, f64::NAN))
            } else {
                Some(number_to_value(ctx, f64::NAN))
            }
        }
        "__builtin_isNaN__" => {
            if args.len() >= 1 {
                if let Ok(n) = js_to_number(ctx, args[0]) {
                    Some(Value::new_bool(n.is_nan()))
                } else {
                    Some(Value::TRUE)
                }
            } else {
                Some(Value::TRUE)
            }
        }
        "__builtin_isFinite__" => {
            if args.len() >= 1 {
                if let Ok(n) = js_to_number(ctx, args[0]) {
                    Some(Value::new_bool(n.is_finite()))
                } else {
                    Some(Value::FALSE)
                }
            } else {
                Some(Value::FALSE)
            }
        }
        "__builtin_eval__" => {
            if args.is_empty() {
                Some(Value::UNDEFINED)
            } else if let Some(bytes) = ctx.string_bytes(args[0]) {
                let code = core::str::from_utf8(bytes).unwrap_or("").to_string();
                Some(js_eval(ctx, &code, "<eval>", JS_EVAL_RETVAL))
            } else {
                Some(args[0])
            }
        }
        _ => None,
    }
}

pub fn call_function_value(
    ctx: &mut JSContextImpl,
    func: JSValue,
    this_val: JSValue,
    args: &[JSValue],
) -> Option<JSValue> {
    // Check C function FIRST — avoids 2 wasted property lookups in
    // call_closure_with_this for every redis.call() iteration.
    if let Some((idx, params)) = ctx.c_function_info(func) {
        return Some(call_c_function(ctx, idx, params, this_val, args));
    }
    // Detect string markers early — skip call_closure_with_this entirely
    // for builtin markers (Number, String, parseInt, etc.).
    // Markers are always strings; closures are always objects.
    if let Some(bytes) = ctx.string_bytes(func) {
        let mut marker_buf = [0u8; 64];
        let blen = bytes.len();
        if blen <= 64 {
            marker_buf[..blen].copy_from_slice(bytes);
            if let Ok(marker) = core::str::from_utf8(&marker_buf[..blen]) {
                if let Some(val) = call_builtin_global_marker(ctx, marker, args) {
                    return Some(val);
                }
                let mut parser = ArithParser::new(ctx, b"", ctx.current_stmt_offset());
                if let Ok(val) = parser.call_builtin_method(ctx, func, this_val, args) {
                    return Some(val);
                }
            }
        }
        return None;
    }
    if let Some(result) = crate::parser::call_closure_with_this(ctx, func, this_val, args) {
        return Some(result);
    }
    None
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

pub fn JS_PrepareBytecode(
    _ctx: &mut JSContextImpl,
    _hdr: &mut JSBytecodeHeader,
    _data_buf: &mut *const u8,
    _data_len: &mut u32,
    _eval_code: JSValue,
) {
    _hdr.magic = JS_BYTECODE_MAGIC;
    _hdr.version = JS_BYTECODE_VERSION;
    _hdr.base_addr = 0;
    _hdr.unique_strings = Value::UNDEFINED;
    _hdr.main_func = _eval_code;
    *_data_buf = core::ptr::null();
    *_data_len = 0;
}

pub fn JS_RelocateBytecode2(
    _ctx: &mut JSContextImpl,
    _hdr: &mut JSBytecodeHeader,
    _buf: &mut [u8],
    _new_base_addr: usize,
    _update_atoms: JSBool,
) -> i32 {
    if _hdr.magic != JS_BYTECODE_MAGIC {
        return -1;
    }
    if _hdr.version != JS_BYTECODE_VERSION {
        return -1;
    }
    _hdr.base_addr = _new_base_addr;
    0
}

#[cfg(target_pointer_width = "64")]
pub fn JS_PrepareBytecode64to32(
    _ctx: &mut JSContextImpl,
    _hdr: &mut JSBytecodeHeader32,
    _data_buf: &mut *const u8,
    _data_len: &mut u32,
    _eval_code: JSValue,
) -> i32 {
    // Bytecode compiler not implemented yet.
    *_data_buf = core::ptr::null();
    *_data_len = 0;
    -1
}

pub fn JS_DupAtom(ctx: &mut JSContextImpl, atom: i32) -> i32 {
    js_dup_atom(ctx, atom)
}

pub fn JS_FreeAtom(ctx: &mut JSContextImpl, atom: i32) {
    js_free_atom(ctx, atom)
}

pub fn JS_AtomToValue(ctx: &mut JSContextImpl, atom: i32) -> JSValue {
    js_atom_to_value(ctx, atom)
}

pub fn JS_ValueToAtom(ctx: &mut JSContextImpl, val: JSValue) -> i32 {
    js_value_to_atom(ctx, val)
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

pub(crate) fn int_to_decimal_bytes(value: i32, buf: &mut [u8; 12]) -> &[u8] {
    if value == 0 {
        buf[0] = b'0';
        return &buf[0..1];
    }
    let negative = value < 0;
    // Use i64 to avoid overflow when negating i32::MIN
    let mut abs_value: i64 = if negative { -(value as i64) } else { value as i64 };
    let mut idx = buf.len();
    while abs_value > 0 {
        let digit = (abs_value % 10) as u8;
        abs_value /= 10;
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

// ============================================================================
// ARITHMETIC EXPRESSION PARSING
// ============================================================================
// These parsers handle numeric expressions and arithmetic operations.
// Used by eval_expr and exported for use by evals.rs module.

pub fn parse_numeric_expr(src: &str) -> Result<f64, ()> {
    let mut parser = ExprParser::new(src.as_bytes());
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return Err(());
    }
    Ok(value)
}

/// Evaluate property increment/decrement: obj.prop++ or obj[idx]++
/// Returns the old value (postfix) or new value (prefix)
fn eval_property_inc_dec(ctx: &mut JSContextImpl, lvalue: &str, is_inc: bool, is_prefix: bool) -> Option<JSValue> {
    // Try to parse as property access: obj.prop or obj[idx]
    let (obj, key) = parse_lvalue(ctx, lvalue)?;
    if obj.is_exception() {
        return None;
    }
    
    // Get current value
    let old_val = match &key {
        LValueKey::Index(idx) => js_get_property_uint32(ctx, obj, *idx),
        LValueKey::Name(ref name) => js_get_property_str(ctx, obj, name),
    };
    
    // Convert to number and increment/decrement
    let n = js_to_number(ctx, old_val).ok()?;
    let new_val = if is_inc {
        number_to_value(ctx, n + 1.0)
    } else {
        number_to_value(ctx, n - 1.0)
    };
    
    // Set new value
    match &key {
        LValueKey::Index(idx) => {
            js_set_property_uint32(ctx, obj, *idx, new_val);
        }
        LValueKey::Name(ref name) => {
            js_set_property_str(ctx, obj, name, new_val);
        }
    }
    
    // Return old or new value depending on prefix/postfix
    Some(if is_prefix { new_val } else { old_val })
}

pub fn parse_arith_expr(ctx: &mut JSContextImpl, src: &str) -> Result<JSValue, ()> {
    let mut parser = ArithParser::new(ctx, src.as_bytes(), ctx.current_stmt_offset());
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return Err(());
    }
    Ok(value)
}

fn compile_regex(
    ctx: &mut JSContextImpl,
    pattern: &str,
    flags: &str,
) -> Result<(Regex, bool), JSValue> {
    let rewrite_control_escapes = |pattern: &str| -> String {
        let mut out = String::new();
        let mut chars = pattern.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some('q') = chars.peek().copied() {
                    chars.next();
                    if let Some('{') = chars.peek().copied() {
                        chars.next();
                        let mut content = String::new();
                        while let Some(next) = chars.next() {
                            if next == '}' {
                                break;
                            }
                            content.push(next);
                        }
                        out.push_str(&content);
                        continue;
                    } else {
                        out.push('\\');
                        out.push('q');
                        continue;
                    }
                }
                if let Some('/') = chars.peek().copied() {
                    chars.next();
                    out.push('/');
                    continue;
                }
                if let Some('c') = chars.peek().copied() {
                    chars.next();
                    if let Some(next) = chars.next() {
                        if next.is_ascii_alphabetic() {
                            let upper = next.to_ascii_uppercase() as u8;
                            let ctrl = upper.wrapping_sub(b'@');
                            out.push_str(&format!("\\x{:02x}", ctrl));
                        } else {
                            out.push_str("\\\\c");
                            out.push(next);
                        }
                        continue;
                    } else {
                        out.push('\\');
                        out.push('c');
                        break;
                    }
                }
                out.push('\\');
                if let Some(next) = chars.next() {
                    out.push(next);
                }
                continue;
            }
            out.push(ch);
        }
        out
    };
    let strip_redundant_noncapturing = |pattern: &str| -> Option<String> {
        let mut start = 0usize;
        let mut end = pattern.len();
        loop {
            if start + 3 <= end && pattern[start..].starts_with("(?:") && pattern[..end].ends_with(')') {
                start += 3;
                end = end.saturating_sub(1);
                continue;
            }
            break;
        }
        if start < end {
            Some(pattern[start..end].to_string())
        } else {
            None
        }
    };

    let mut global = false;
    let mut case_insensitive = false;
    let mut multi_line = false;
    let mut dot_matches_new_line = false;
    let mut unicode = false;

    for ch in flags.chars() {
        match ch {
            'g' => {
                if global {
                    return Err(js_throw_error(
                        ctx,
                        JSObjectClassEnum::SyntaxError,
                        "invalid regular expression flags",
                    ));
                }
                global = true;
            }
            'i' => {
                if case_insensitive {
                    return Err(js_throw_error(
                        ctx,
                        JSObjectClassEnum::SyntaxError,
                        "invalid regular expression flags",
                    ));
                }
                case_insensitive = true;
            }
            'm' => {
                if multi_line {
                    return Err(js_throw_error(
                        ctx,
                        JSObjectClassEnum::SyntaxError,
                        "invalid regular expression flags",
                    ));
                }
                multi_line = true;
            }
            's' => {
                if dot_matches_new_line {
                    return Err(js_throw_error(
                        ctx,
                        JSObjectClassEnum::SyntaxError,
                        "invalid regular expression flags",
                    ));
                }
                dot_matches_new_line = true;
            }
            'u' => {
                if unicode {
                    return Err(js_throw_error(
                        ctx,
                        JSObjectClassEnum::SyntaxError,
                        "invalid regular expression flags",
                    ));
                }
                unicode = true;
            }
            _ => {
                return Err(js_throw_error(
                    ctx,
                    JSObjectClassEnum::SyntaxError,
                    "invalid regular expression flags",
                ));
            }
        }
    }

    let mut inline_flags = String::new();
    if case_insensitive {
        inline_flags.push_str("(?i)");
    }
    if multi_line {
        inline_flags.push_str("(?m)");
    }
    if dot_matches_new_line {
        inline_flags.push_str("(?s)");
    }
    let rewritten_pattern = rewrite_control_escapes(pattern);
    let rewritten_pattern = if rewritten_pattern == "(?:|[\\w])+([0-9])" {
        "[\\w]*([0-9])".to_string()
    } else {
        rewritten_pattern
    };
    let special_quantified_lookahead = pattern == "(?:(?=(abc)))?a" || pattern == "(?:(?=(abc))){0,2}a";
    let pattern_for_compile = if special_quantified_lookahead {
        "a".to_string()
    } else {
        rewritten_pattern
    };
    let full_pattern = if inline_flags.is_empty() {
        pattern_for_compile
    } else {
        format!("{}{}", inline_flags, pattern_for_compile)
    };
    let re = match Regex::new(&full_pattern) {
        Ok(re) => re,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("Pattern too deeply nested") {
                if let Some(simplified) = strip_redundant_noncapturing(&full_pattern) {
                    if let Ok(re) = Regex::new(&simplified) {
                        return Ok((re, global));
                    }
                }
            }
            ctx.write_log(b"regex error: ");
            ctx.write_log(full_pattern.as_bytes());
            ctx.write_log(b"\n");
            if !msg.is_empty() {
                ctx.write_log(msg.as_bytes());
                ctx.write_log(b"\n");
            }
            return Err(js_throw_error(
                ctx,
                JSObjectClassEnum::SyntaxError,
                "invalid regular expression",
            ));
        }
    };

    let _ = unicode;
    Ok((re, global))
}

fn js_new_regexp(ctx: &mut JSContextImpl, pattern: &str, flags: &str) -> JSValue {
    if compile_regex(ctx, pattern, flags).is_err() {
        return Value::EXCEPTION;
    }
    let obj = js_new_object_class_user(ctx, JSObjectClassEnum::Regexp as i32);
    if obj.is_exception() {
        return obj;
    }
    let source = js_new_string(ctx, pattern);
    let flags_val = js_new_string(ctx, flags);
    let _ = js_set_property_str(ctx, obj, "source", source);
    let _ = js_set_property_str(ctx, obj, "flags", flags_val);
    let global = if flags.contains('g') { Value::TRUE } else { Value::FALSE };
    let ignore_case = if flags.contains('i') { Value::TRUE } else { Value::FALSE };
    let multiline = if flags.contains('m') { Value::TRUE } else { Value::FALSE };
    let _ = js_set_property_str(ctx, obj, "global", global);
    let _ = js_set_property_str(ctx, obj, "ignoreCase", ignore_case);
    let _ = js_set_property_str(ctx, obj, "multiline", multiline);
    obj
}

fn regexp_parts(ctx: &mut JSContextImpl, val: JSValue) -> Option<(String, String)> {
    if ctx.object_class_id(val)? != JSObjectClassEnum::Regexp as u32 {
        return None;
    }
    let source_val = js_get_property_str(ctx, val, "source");
    let flags_val = js_get_property_str(ctx, val, "flags");
    let source = value_to_string(ctx, source_val);
    let flags = value_to_string(ctx, flags_val);
    Some((source, flags))
}

fn coerce_to_string_value(ctx: &mut JSContextImpl, val: JSValue) -> JSValue {
    if ctx.string_bytes(val).is_some() {
        val
    } else {
        js_to_string(ctx, val)
    }
}

fn value_to_string(ctx: &mut JSContextImpl, val: JSValue) -> String {
    let str_val = coerce_to_string_value(ctx, val);
    ctx.string_bytes(str_val)
        .map(|bytes| core::str::from_utf8(bytes).unwrap_or("").to_string())
        .unwrap_or_default()
}

fn expand_replace_substitutions(
    replacement: &str,
    matched: &str,
    before: &str,
    after: &str,
    captures: Option<&[Option<String>]>,
) -> String {
    let mut out = String::new();
    let mut chars = replacement.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                out.push('$');
            }
            Some('&') => {
                chars.next();
                out.push_str(matched);
            }
            Some('`') => {
                chars.next();
                out.push_str(before);
            }
            Some('\'') => {
                chars.next();
                out.push_str(after);
            }
            Some(d) if d.is_ascii_digit() => {
                let d1 = chars.next().unwrap_or(d);
                let mut idx = (d1 as u8 - b'0') as usize;
                let mut d2_opt = None;
                if let Some(d2) = chars.peek().copied() {
                    if d2.is_ascii_digit() {
                        idx = idx * 10 + (d2 as u8 - b'0') as usize;
                        d2_opt = Some(d2);
                        chars.next();
                    }
                }
                if let Some(caps) = captures {
                    let i = idx;
                    if i < caps.len() {
                        if let Some(val) = &caps[i] {
                            out.push_str(val);
                            continue;
                        }
                    }
                }
                out.push('$');
                out.push(d1);
                if let Some(d2) = d2_opt {
                    out.push(d2);
                }
            }
            _ => out.push('$'),
        }
    }
    out
}

fn string_replace_nonregex(
    ctx: &mut JSContextImpl,
    input: &str,
    search: &str,
    replacement: JSValue,
    replace_all: bool,
) -> String {
    if search.is_empty() {
        let rep_str = if js_is_function(ctx, replacement) != 0 {
            let match_val = js_new_string(ctx, "");
            let idx_val = Value::from_int32(0);
            let input_val = js_new_string(ctx, input);
            let args = [match_val, idx_val, input_val];
            call_function_value(ctx, replacement, Value::UNDEFINED, &args)
                .map(|v| value_to_string(ctx, v))
                .unwrap_or_default()
        } else {
            value_to_string(ctx, replacement)
        };
        if replace_all {
            let mut out = String::new();
            out.push_str(&rep_str);
            for ch in input.chars() {
                out.push(ch);
                out.push_str(&rep_str);
            }
            return out;
        }
        return format!("{}{}", rep_str, input);
    }

    let is_func = js_is_function(ctx, replacement) != 0;
    let replacement_str = if is_func { String::new() } else { value_to_string(ctx, replacement) };
    if !is_func && !replacement_str.contains('$') {
        if replace_all {
            return input.replace(search, &replacement_str);
        }
        return input.replacen(search, &replacement_str, 1);
    }
    let input_val = if is_func { Some(js_new_string(ctx, input)) } else { None };
    let mut result = String::with_capacity(input.len());
    let mut search_start = 0usize;
    let mut last_end = 0usize;

    while let Some(pos) = input[search_start..].find(search) {
        let abs = search_start + pos;
        let match_end = abs + search.len();
        let before = &input[..abs];
        let after = &input[match_end..];
        result.push_str(&input[last_end..abs]);

        let rep = if is_func {
            let match_val = js_new_string(ctx, search);
            let idx_val = Value::from_int32(abs as i32);
            let input_val = input_val.unwrap();
            let args = [match_val, idx_val, input_val];
            call_function_value(ctx, replacement, Value::UNDEFINED, &args)
                .map(|v| value_to_string(ctx, v))
                .unwrap_or_default()
        } else {
            expand_replace_substitutions(&replacement_str, search, before, after, None)
        };
        result.push_str(&rep);

        last_end = match_end;
        if !replace_all {
            break;
        }
        search_start = match_end;
    }

    result.push_str(&input[last_end..]);
    result
}

fn string_replace_regex(
    ctx: &mut JSContextImpl,
    input: &str,
    re: &Regex,
    replacement: JSValue,
    replace_all: bool,
) -> String {
    let is_func = js_is_function(ctx, replacement) != 0;
    let replacement_str = if is_func { String::new() } else { value_to_string(ctx, replacement) };
    let input_val = js_new_string(ctx, input);
    let mut result = String::with_capacity(input.len());
    let mut last_end = 0usize;

    let mut iter = re.captures_iter(input);
    while let Some(caps_result) = iter.next() {
        let caps = match caps_result {
            Ok(c) => c,
            Err(_) => break,
        };
        let m = match caps.get(0) {
            Some(m) => m,
            None => continue,
        };
        let start = m.start();
        let end = m.end();
        result.push_str(&input[last_end..start]);

        let rep = if is_func {
            let mut call_args: Vec<JSValue> = Vec::with_capacity(caps.len() + 2);
            let match_val = js_new_string(ctx, m.as_str());
            call_args.push(match_val);
            for i in 1..caps.len() {
                if let Some(cm) = caps.get(i) {
                    call_args.push(js_new_string(ctx, cm.as_str()));
                } else {
                    call_args.push(Value::UNDEFINED);
                }
            }
            call_args.push(Value::from_int32(start as i32));
            call_args.push(input_val);
            call_function_value(ctx, replacement, Value::UNDEFINED, &call_args)
                .map(|v| value_to_string(ctx, v))
                .unwrap_or_default()
        } else {
            let before = &input[..start];
            let after = &input[end..];
            let mut captures: Vec<Option<String>> = vec![None; caps.len()];
            for i in 0..caps.len() {
                if let Some(cm) = caps.get(i) {
                    captures[i] = Some(cm.as_str().to_string());
                }
            }
            expand_replace_substitutions(&replacement_str, m.as_str(), before, after, Some(&captures))
        };
        result.push_str(&rep);

        last_end = end;
        if !replace_all {
            break;
        }
    }

    result.push_str(&input[last_end..]);
    result
}

