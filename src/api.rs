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
use crate::helpers::{number_to_value, is_identifier, flatten_array, contains_arith_op};
use crate::json::parse_json;
use crate::evals::{
    eval_value,
    split_top_level,
    split_statements,
    strip_comments,
    normalize_line_continuations,
    is_truthy,
};
use crate::parser::*;

fn string_utf16_units(ctx: &mut JSContextImpl, val: JSValue) -> Option<Vec<u16>> {
    let bytes = ctx.string_bytes(val)?;
    let s = core::str::from_utf8(bytes).ok()?;
    Some(s.encode_utf16().collect())
}

fn string_utf16_len(ctx: &mut JSContextImpl, val: JSValue) -> Option<usize> {
    string_utf16_units(ctx, val).map(|units| units.len())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TypedArrayKind {
    Uint8,
    Uint8Clamped,
    Int8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float32,
    Float64,
}

impl TypedArrayKind {
    fn elem_size(self) -> usize {
        match self {
            TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped | TypedArrayKind::Int8 => 1,
            TypedArrayKind::Int16 | TypedArrayKind::Uint16 => 2,
            TypedArrayKind::Int32 | TypedArrayKind::Uint32 | TypedArrayKind::Float32 => 4,
            TypedArrayKind::Float64 => 8,
        }
    }
}

#[derive(Debug)]
struct ArrayBufferData {
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct TypedArrayData {
    buffer: JSValue,
    offset: usize,
    length: usize,
    kind: TypedArrayKind,
}

fn typed_array_kind_from_class_id(class_id: u32) -> Option<TypedArrayKind> {
    match class_id {
        x if x == JSObjectClassEnum::Uint8Array as u32 => Some(TypedArrayKind::Uint8),
        x if x == JSObjectClassEnum::Uint8cArray as u32 => Some(TypedArrayKind::Uint8Clamped),
        x if x == JSObjectClassEnum::Int8Array as u32 => Some(TypedArrayKind::Int8),
        x if x == JSObjectClassEnum::Int16Array as u32 => Some(TypedArrayKind::Int16),
        x if x == JSObjectClassEnum::Uint16Array as u32 => Some(TypedArrayKind::Uint16),
        x if x == JSObjectClassEnum::Int32Array as u32 => Some(TypedArrayKind::Int32),
        x if x == JSObjectClassEnum::Uint32Array as u32 => Some(TypedArrayKind::Uint32),
        x if x == JSObjectClassEnum::Float32Array as u32 => Some(TypedArrayKind::Float32),
        x if x == JSObjectClassEnum::Float64Array as u32 => Some(TypedArrayKind::Float64),
        _ => None,
    }
}

fn typed_array_class_enum_from_id(class_id: u32) -> Option<JSObjectClassEnum> {
    match class_id {
        x if x == JSObjectClassEnum::Uint8Array as u32 => Some(JSObjectClassEnum::Uint8Array),
        x if x == JSObjectClassEnum::Uint8cArray as u32 => Some(JSObjectClassEnum::Uint8cArray),
        x if x == JSObjectClassEnum::Int8Array as u32 => Some(JSObjectClassEnum::Int8Array),
        x if x == JSObjectClassEnum::Int16Array as u32 => Some(JSObjectClassEnum::Int16Array),
        x if x == JSObjectClassEnum::Uint16Array as u32 => Some(JSObjectClassEnum::Uint16Array),
        x if x == JSObjectClassEnum::Int32Array as u32 => Some(JSObjectClassEnum::Int32Array),
        x if x == JSObjectClassEnum::Uint32Array as u32 => Some(JSObjectClassEnum::Uint32Array),
        x if x == JSObjectClassEnum::Float32Array as u32 => Some(JSObjectClassEnum::Float32Array),
        x if x == JSObjectClassEnum::Float64Array as u32 => Some(JSObjectClassEnum::Float64Array),
        _ => None,
    }
}

fn get_arraybuffer_data(ctx: &mut JSContextImpl, val: JSValue) -> Option<*mut ArrayBufferData> {
    if ctx.object_class_id(val)? != JSObjectClassEnum::ArrayBuffer as u32 {
        return None;
    }
    let ptr = ctx.get_object_opaque(val) as *mut ArrayBufferData;
    if ptr.is_null() { None } else { Some(ptr) }
}

fn get_typedarray_data(ctx: &mut JSContextImpl, val: JSValue) -> Option<*mut TypedArrayData> {
    let class_id = ctx.object_class_id(val)?;
    if typed_array_kind_from_class_id(class_id).is_none() {
        return None;
    }
    let ptr = ctx.get_object_opaque(val) as *mut TypedArrayData;
    if ptr.is_null() { None } else { Some(ptr) }
}

fn clamp_u8_clamped(n: f64) -> u8 {
    if !n.is_finite() || n <= 0.0 {
        return 0;
    }
    if n >= 255.0 {
        return 255;
    }
    let floor = n.floor();
    let frac = n - floor;
    let mut rounded = if frac > 0.5 {
        floor + 1.0
    } else if frac < 0.5 {
        floor
    } else {
        if (floor as i64) % 2 == 0 { floor } else { floor + 1.0 }
    };
    if rounded < 0.0 { rounded = 0.0; }
    if rounded > 255.0 { rounded = 255.0; }
    rounded as u8
}

fn typed_array_get_element(ctx: &mut JSContextImpl, obj: JSValue, idx: u32) -> JSValue {
    let data_ptr = match get_typedarray_data(ctx, obj) {
        Some(p) => p,
        None => return Value::UNDEFINED,
    };
    unsafe {
        let data = &*data_ptr;
        if idx as usize >= data.length {
            return Value::UNDEFINED;
        }
        let buf_ptr = match get_arraybuffer_data(ctx, data.buffer) {
            Some(p) => p,
            None => return Value::UNDEFINED,
        };
        let buf = &*buf_ptr;
        let size = data.kind.elem_size();
        let start = data.offset + (idx as usize) * size;
        if start + size > buf.bytes.len() {
            return Value::UNDEFINED;
        }
        let bytes = &buf.bytes[start..start + size];
        match data.kind {
            TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped => Value::from_int32(bytes[0] as i32),
            TypedArrayKind::Int8 => Value::from_int32((bytes[0] as i8) as i32),
            TypedArrayKind::Int16 => {
                let v = i16::from_le_bytes([bytes[0], bytes[1]]);
                Value::from_int32(v as i32)
            }
            TypedArrayKind::Uint16 => {
                let v = u16::from_le_bytes([bytes[0], bytes[1]]);
                Value::from_int32(v as i32)
            }
            TypedArrayKind::Int32 => {
                let v = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                Value::from_int32(v)
            }
            TypedArrayKind::Uint32 => {
                let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if v <= i32::MAX as u32 { Value::from_int32(v as i32) } else { number_to_value(ctx, v as f64) }
            }
            TypedArrayKind::Float32 => {
                let v = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                number_to_value(ctx, v as f64)
            }
            TypedArrayKind::Float64 => {
                let v = f64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]);
                number_to_value(ctx, v)
            }
        }
    }
}

fn typed_array_set_element(ctx: &mut JSContextImpl, obj: JSValue, idx: u32, val: JSValue) -> bool {
    let data_ptr = match get_typedarray_data(ctx, obj) {
        Some(p) => p,
        None => return false,
    };
    unsafe {
        let data = &*data_ptr;
        if idx as usize >= data.length {
            return false;
        }
        let buf_ptr = match get_arraybuffer_data(ctx, data.buffer) {
            Some(p) => p,
            None => return false,
        };
        let buf = &mut *buf_ptr;
        let size = data.kind.elem_size();
        let start = data.offset + (idx as usize) * size;
        if start + size > buf.bytes.len() {
            return false;
        }
        let n = js_to_number(ctx, val).unwrap_or(0.0);
        match data.kind {
            TypedArrayKind::Uint8 => {
                let mut v = n.trunc() as i64;
                v = v.rem_euclid(256);
                buf.bytes[start] = v as u8;
            }
            TypedArrayKind::Uint8Clamped => {
                buf.bytes[start] = clamp_u8_clamped(n);
            }
            TypedArrayKind::Int8 => {
                let mut v = n.trunc() as i64;
                v = v.rem_euclid(256);
                if v >= 128 { v -= 256; }
                buf.bytes[start] = (v as i8) as u8;
            }
            TypedArrayKind::Int16 => {
                let mut v = n.trunc() as i64;
                v = v.rem_euclid(1 << 16);
                if v >= (1 << 15) { v -= 1 << 16; }
                let bytes = (v as i16).to_le_bytes();
                buf.bytes[start..start + 2].copy_from_slice(&bytes);
            }
            TypedArrayKind::Uint16 => {
                let mut v = n.trunc() as i64;
                v = v.rem_euclid(1 << 16);
                let bytes = (v as u16).to_le_bytes();
                buf.bytes[start..start + 2].copy_from_slice(&bytes);
            }
            TypedArrayKind::Int32 => {
                let v = js_to_int32(ctx, val).unwrap_or(0);
                let bytes = v.to_le_bytes();
                buf.bytes[start..start + 4].copy_from_slice(&bytes);
            }
            TypedArrayKind::Uint32 => {
                let v = js_to_uint32(ctx, val).unwrap_or(0);
                let bytes = v.to_le_bytes();
                buf.bytes[start..start + 4].copy_from_slice(&bytes);
            }
            TypedArrayKind::Float32 => {
                let v = n as f32;
                let bytes = v.to_le_bytes();
                buf.bytes[start..start + 4].copy_from_slice(&bytes);
            }
            TypedArrayKind::Float64 => {
                let bytes = n.to_le_bytes();
                buf.bytes[start..start + 8].copy_from_slice(&bytes);
            }
        }
    }
    true
}

fn create_arraybuffer(ctx: &mut JSContextImpl, byte_len: usize) -> JSValue {
    let obj = js_new_object_class_user(ctx, JSObjectClassEnum::ArrayBuffer as i32);
    if obj.is_exception() {
        return obj;
    }
    let data = Box::new(ArrayBufferData { bytes: vec![0u8; byte_len] });
    ctx.set_object_opaque(obj, Box::into_raw(data) as *mut core::ffi::c_void);
    let _ = js_set_property_str(ctx, obj, "byteLength", Value::from_int32(byte_len as i32));
    obj
}

fn create_typed_array(ctx: &mut JSContextImpl, class_id: JSObjectClassEnum, buffer: JSValue, offset: usize, length: usize) -> JSValue {
    let obj = js_new_object_class_user(ctx, class_id as i32);
    if obj.is_exception() {
        return obj;
    }
    let kind = typed_array_kind_from_class_id(class_id as u32).unwrap();
    let data = Box::new(TypedArrayData { buffer, offset, length, kind });
    ctx.set_object_opaque(obj, Box::into_raw(data) as *mut core::ffi::c_void);
    let _ = js_set_property_str(ctx, obj, "length", Value::from_int32(length as i32));
    let _ = js_set_property_str(ctx, obj, "byteLength", Value::from_int32((length * kind.elem_size()) as i32));
    let _ = js_set_property_str(ctx, obj, "byteOffset", Value::from_int32(offset as i32));
    let _ = js_set_property_str(ctx, obj, "buffer", buffer);
    let _ = js_set_property_str(ctx, obj, "BYTES_PER_ELEMENT", Value::from_int32(kind.elem_size() as i32));
    obj
}

fn build_typed_array_from_args(ctx: &mut JSContextImpl, class_id: JSObjectClassEnum, args: &[JSValue]) -> JSValue {
    let kind = typed_array_kind_from_class_id(class_id as u32).unwrap();
    let elem_size = kind.elem_size();
    if args.is_empty() {
        let buffer = create_arraybuffer(ctx, 0);
        return create_typed_array(ctx, class_id, buffer, 0, 0);
    }
    // ArrayBuffer overload
    if let Some(class_id0) = ctx.object_class_id(args[0]) {
        if class_id0 == JSObjectClassEnum::ArrayBuffer as u32 {
            let buffer = args[0];
            let buf_ptr = match get_arraybuffer_data(ctx, buffer) {
                Some(p) => p,
                None => return Value::UNDEFINED,
            };
            let byte_len = unsafe { (*buf_ptr).bytes.len() };
            let offset = if args.len() >= 2 { js_to_int32(ctx, args[1]).unwrap_or(0).max(0) as usize } else { 0 };
            let mut length = if args.len() >= 3 {
                js_to_int32(ctx, args[2]).unwrap_or(0).max(0) as usize
            } else {
                (byte_len.saturating_sub(offset)) / elem_size
            };
            if offset > byte_len {
                length = 0;
            }
            return create_typed_array(ctx, class_id, buffer, offset, length);
        }
        if typed_array_kind_from_class_id(class_id0).is_some() {
            // TypedArray from another TypedArray
            let src_len = js_get_property_str(ctx, args[0], "length").int32().unwrap_or(0).max(0) as usize;
            let buffer = create_arraybuffer(ctx, src_len * elem_size);
            let out = create_typed_array(ctx, class_id, buffer, 0, src_len);
            for i in 0..src_len {
                let v = typed_array_get_element(ctx, args[0], i as u32);
                typed_array_set_element(ctx, out, i as u32, v);
            }
            return out;
        }
        if class_id0 == JSObjectClassEnum::Array as u32 {
            let src_len = js_get_property_str(ctx, args[0], "length").int32().unwrap_or(0).max(0) as usize;
            let buffer = create_arraybuffer(ctx, src_len * elem_size);
            let out = create_typed_array(ctx, class_id, buffer, 0, src_len);
            for i in 0..src_len {
                let v = js_get_property_uint32(ctx, args[0], i as u32);
                typed_array_set_element(ctx, out, i as u32, v);
            }
            return out;
        }
    }
    // length overload
    let length = js_to_int32(ctx, args[0]).unwrap_or(0).max(0) as usize;
    let buffer = create_arraybuffer(ctx, length * elem_size);
    create_typed_array(ctx, class_id, buffer, 0, length)
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
    if let Some(id) = _ctx.object_class_id(_val) {
        let func = JSObjectClassEnum::CFunction as u32;
        let closure = JSObjectClassEnum::Closure as u32;
        if id == func || id == closure {
            return 1;
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
    let msg = js_new_string(_ctx, _msg);
    _ctx.set_exception(msg);
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
    if let Some(units) = string_utf16_units(_ctx, _obj) {
        let idx = _idx as usize;
        if idx < units.len() {
            let s = String::from_utf16_lossy(&[units[idx]]);
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
    let src = _input.trim();
    if (_eval_flags & JS_EVAL_JSON) != 0 {
        if let Some(val) = crate::json::parse_json(_ctx, src) {
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
            }
        }
    }
    None
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
            if _ctx.object_class_id(prim).is_none() {
                return js_to_number(_ctx, prim);
            }
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
        let marker_owned = core::str::from_utf8(bytes).ok().map(|s| s.to_string());
        if let Some(marker) = marker_owned.as_deref() {
            if let Some(val) = call_builtin_global_marker(_ctx, marker, &args) {
                return val;
            }
        }
        let mut parser = ArithParser::new(_ctx, b"");
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

fn int_to_decimal_bytes(value: i32, buf: &mut [u8; 12]) -> &[u8] {
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
    let mut parser = ArithParser::new(ctx, src.as_bytes());
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() {
        return Err(());
    }
    Ok(value)
}

fn normalize_exponent(s: &str) -> String {
    let (base, exp) = match s.find('e').or_else(|| s.find('E')) {
        Some(idx) => (&s[..idx], &s[idx + 1..]),
        None => return s.to_string(),
    };
    let mut sign = '+';
    let mut digits = exp;
    if let Some(rest) = exp.strip_prefix('-') {
        sign = '-';
        digits = rest;
    } else if let Some(rest) = exp.strip_prefix('+') {
        digits = rest;
    }
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    format!("{}e{}{}", base, sign, digits)
}

fn format_fixed(n: f64, digits: i32) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    let prec = digits.max(0) as i32;
    let factor = 10_f64.powi(prec);
    let rounded = round_half_away_from_zero(n * factor) / factor;
    if rounded == 0.0 && n.is_sign_negative() {
        if prec > 0 {
            return format!("-0.{:0width$}", 0, width = prec as usize);
        }
        return "-0".to_string();
    }
    format!("{:.*}", prec as usize, rounded)
}

fn format_exponential(n: f64, digits: Option<i32>) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    if let Some(d) = digits {
        return format_exponential_rounded(n, d);
    }
    let s = format!("{:e}", n);
    normalize_exponent(&s)
}

fn format_radix_int(value: i64, radix: u32) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if radix < 2 || radix > 36 {
        return String::new();
    }
    if value == 0 {
        return "0".to_string();
    }
    let mut n = value as i128;
    let negative = n < 0;
    if negative {
        n = -n;
    }
    let radix_i = radix as i128;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n % radix_i) as usize;
        out.push(digits[rem]);
        n /= radix_i;
    }
    if negative {
        out.push(b'-');
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn format_precision(n: f64, precision: i32) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    if n == 0.0 {
        let mut out = String::from("0");
        if precision > 1 {
            out.push('.');
            for _ in 1..precision {
                out.push('0');
            }
        }
        return out;
    }
    let abs = n.abs();
    let exp = abs.log10().floor() as i32;
    if exp < -6 || exp >= precision {
        return format_exponential(n, Some(precision - 1));
    }
    let frac = (precision - exp - 1).max(0) as i32;
    let factor = 10_f64.powi(frac);
    let rounded = round_half_away_from_zero(n * factor) / factor;
    if rounded == 0.0 && n.is_sign_negative() {
        if frac > 0 {
            return format!("-0.{:0width$}", 0, width = frac as usize);
        }
        return "-0".to_string();
    }
    format!("{:.*}", frac as usize, rounded)
}

fn round_half_away_from_zero(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() {
        return n;
    }
    if n.is_sign_negative() {
        return -round_half_away_from_zero(-n);
    }
    let floor = n.floor();
    let frac = n - floor;
    if frac > 0.5 {
        floor + 1.0
    } else if frac < 0.5 {
        floor
    } else {
        floor + 1.0
    }
}

fn format_exponential_rounded(n: f64, digits: i32) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    if n == 0.0 {
        let mut s = String::from("0");
        if digits > 0 {
            s.push('.');
            s.push_str(&"0".repeat(digits as usize));
        }
        s.push_str("e+0");
        return s;
    }
    let sign = if n.is_sign_negative() { "-" } else { "" };
    let abs = n.abs();
    let mut exp = abs.log10().floor() as i32;
    let mut normalized = abs / 10_f64.powi(exp);
    let factor = 10_f64.powi(digits);
    let rounded = round_half_away_from_zero(normalized * factor);
    normalized = rounded / factor;
    if normalized >= 10.0 {
        normalized /= 10.0;
        exp += 1;
    }
    let mut out = format!("{:.*}", digits as usize, normalized);
    out = normalize_exponent(&format!("{}{}e{:+}", sign, out, exp));
    out
}

fn compile_regex(
    ctx: &mut JSContextImpl,
    pattern: &str,
    flags: &str,
) -> Result<(Regex, bool), JSValue> {
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
    let full_pattern = if inline_flags.is_empty() {
        pattern.to_string()
    } else {
        format!("{}{}", inline_flags, pattern)
    };
    let re = Regex::new(&full_pattern).map_err(|_| {
        js_throw_error(
            ctx,
            JSObjectClassEnum::SyntaxError,
            "invalid regular expression",
        )
    })?;

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
    let source = ctx
        .string_bytes(source_val)
        .map(|bytes| core::str::from_utf8(bytes).unwrap_or("").to_string())
        .unwrap_or_default();
    let flags = ctx
        .string_bytes(flags_val)
        .map(|bytes| core::str::from_utf8(bytes).unwrap_or("").to_string())
        .unwrap_or_default();
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

// ============================================================================
// EXPRESSION EVALUATION
// ============================================================================
// NOTE: Core evaluation utilities have been extracted to evals.rs:
// - eval_value() - Also available in evals.rs as public API
// - eval_array_literal() - Extracted to evals.rs
// - eval_object_literal() - Extracted to evals.rs
// - split_top_level() - Extracted to evals.rs
// - split_statements() - Extracted to evals.rs
//
// The massive eval_expr() function (~2,600 lines) remains here because it
// contains 83 inline built-in method handlers that are tightly coupled.

// ============================================================================
// MAIN EXPRESSION EVALUATOR (eval_expr)
// ============================================================================
// This ~2,600 line function handles:
// - Variable declarations and assignments
// - Operators (arithmetic, comparison, logical, ternary)
// - Property access and method calls
// - 83 built-in method implementations (String.*, Array.*, Object.*, Math.*, etc.)
//
// Built-in methods are implemented inline using marker strings like:
// "__builtin_string_charAt__", "__builtin_array_map__", etc.
//
// Future refactoring: Extract built-ins to separate handlers (Phase 2)

/// Parse a simple member access expression: obj.prop
/// Returns (object_expression, property_name) if successful
fn parse_member_access(src: &str) -> Option<(&str, String)> {
    let s = src.trim();
    // Find the last dot that's not inside brackets or parens
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut last_dot = None;
    for i in 0..bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == string_delim {
                in_string = false;
            } else if b == b'\\' && i + 1 < bytes.len() {
                continue; // skip next char
            }
            continue;
        }
        if b == b'\'' || b == b'"' {
            in_string = true;
            string_delim = b;
            continue;
        }
        match b {
            b'[' | b'{' | b'(' => depth += 1,
            b']' | b'}' | b')' => depth -= 1,
            b'.' if depth == 0 => last_dot = Some(i),
            _ => {}
        }
    }
    if let Some(dot_pos) = last_dot {
        let obj_part = s[..dot_pos].trim();
        let prop_part = s[dot_pos + 1..].trim();
        if !obj_part.is_empty() && is_identifier(prop_part) {
            return Some((obj_part, prop_part.to_string()));
        }
    }
    None
}

pub fn eval_expr(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    if s.is_empty() {
        return None;
    }
    // Handle var/let/const declarations: var x = expr OR var x
    if s.starts_with("var ") || s.starts_with("let ") || s.starts_with("const ") {
        let (kind, rest) = if s.starts_with("var ") {
            ("var", s[4..].trim())
        } else if s.starts_with("let ") {
            ("let", s[4..].trim())
        } else {
            ("const", s[6..].trim())
        };
        let decls = split_top_level(rest).unwrap_or_else(|| vec![rest]);
        let env = if kind == "var" {
            ctx.current_var_env()
        } else {
            ctx.current_env()
        };
        for decl in decls {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            if let Some(eq_pos) = decl.find('=') {
                let var_name = decl[..eq_pos].trim();
                let init_expr = decl[eq_pos + 1..].trim();
                if is_identifier(var_name) {
                    let val = eval_expr(ctx, init_expr)?;
                    js_set_property_str(ctx, env, var_name, val);
                    if kind == "const" {
                        mark_const_binding(ctx, env, var_name);
                    }
                    continue;
                } else {
                    return None;
                }
            } else {
                if !is_identifier(decl) {
                    return None;
                }
                if kind == "const" {
                    return Some(js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "const declarations require initialization"));
                }
                js_set_property_str(ctx, env, decl, Value::UNDEFINED);
            }
        }
        return Some(Value::UNDEFINED);
    }
    // Comma operator (lowest precedence)
    if s.contains(',') {
        if let Some(parts) = split_top_level(s) {
            if parts.len() > 1 {
                let mut last = Value::UNDEFINED;
                for part in parts {
                    last = eval_expr(ctx, part)?;
                }
                return Some(last);
            }
        }
    }
    if s.contains("=>") && has_top_level_arrow(s) {
        if let Some(val) = eval_value(ctx, s) {
            return Some(val);
        }
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
        if let LValueKey::Name(name) = &key {
            if is_const_binding(ctx, base, name) {
                let current = js_get_property_str(ctx, base, name);
                if current != Value::UNINITIALIZED {
                    return Some(js_throw_error(
                        ctx,
                        JSObjectClassEnum::TypeError,
                        "invalid assignment to const",
                    ));
                }
            }
        }
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
        let is_true = is_truthy(ctx, cond_val);
        if is_true {
            return eval_expr(ctx, true_part);
        } else {
            return eval_expr(ctx, false_part);
        }
    }
    // Check for instanceof operator
    if let Some((lhs, rhs)) = split_instanceof(s) {
        let left = eval_expr(ctx, lhs)?;
        let right = eval_expr(ctx, rhs)?;
        let mut result = false;
        
        // Check if right is a builtin marker string
        if let Some(bytes) = ctx.string_bytes(right) {
            if let Ok(marker) = core::str::from_utf8(bytes) {
                result = match marker {
                    "__builtin_Error__" => js_is_error(ctx, left) != 0,
                    "__builtin_TypeError__" => ctx.object_class_id(left) == Some(JSObjectClassEnum::TypeError as u32),
                    "__builtin_ReferenceError__" => ctx.object_class_id(left) == Some(JSObjectClassEnum::ReferenceError as u32),
                    "__builtin_SyntaxError__" => ctx.object_class_id(left) == Some(JSObjectClassEnum::SyntaxError as u32),
                    "__builtin_RangeError__" => ctx.object_class_id(left) == Some(JSObjectClassEnum::RangeError as u32),
                    "__builtin_Array__" => ctx.object_class_id(left) == Some(JSObjectClassEnum::Array as u32),
                    "__builtin_Object__" => ctx.object_class_id(left).is_some(),
                    _ => false,
                };
                return Some(Value::new_bool(result));
            }
        }
        
        // For regular constructor functions, check prototype chain
        // Get the constructor's prototype property
        let ctor_proto = js_get_property_str(ctx, right, "prototype");
        if ctor_proto.is_undefined() || ctor_proto.is_null() {
            // Not a valid constructor, return false
            return Some(Value::FALSE);
        }
        
        // Walk the prototype chain of left
        let mut proto = js_get_property_str(ctx, left, "__proto__");
        while !proto.is_undefined() && !proto.is_null() {
            // Use raw value comparison
            if proto.0 == ctor_proto.0 {
                return Some(Value::TRUE);
            }
            proto = js_get_property_str(ctx, proto, "__proto__");
        }
        
        return Some(Value::FALSE);
    }
    // Check for `in` operator (property existence check)
    if let Some((lhs, rhs)) = split_in_operator(s) {
        // Left side is the property name (as string), right side is the object
        let prop_val = eval_expr(ctx, lhs)?;
        let obj_val = eval_expr(ctx, rhs)?;

        // Get property name as string
        let prop_name = if let Some(bytes) = ctx.string_bytes(prop_val) {
            core::str::from_utf8(bytes).ok().map(|s| s.to_string())
        } else if let Some(n) = prop_val.int32() {
            Some(n.to_string())
        } else {
            None
        };

        if let Some(name) = prop_name {
            // Check if object has the property (including prototype chain)
            let has_prop = ctx.has_property_str(obj_val, name.as_bytes()) ||
                js_get_property_str(ctx, obj_val, &name) != Value::UNDEFINED;
            return Some(Value::new_bool(has_prop));
        }
        return Some(Value::FALSE);
    }
    // Check for arithmetic operators before splitting on base/tail
    if contains_arith_op(s) {
        if let Ok(val) = parse_arith_expr(ctx, s) {
            return Some(val);
        }
    }
    // Check for postfix ++ or --
    if s.ends_with("++") || s.ends_with("--") {
        let lvalue_str = s[..s.len() - 2].trim();
        let is_inc = s.ends_with("++");
        // Try property access: obj.prop++ or obj[idx]++
        if let Some(result) = eval_property_inc_dec(ctx, lvalue_str, is_inc, false) {
            return Some(result);
        }
        // Simple variable
        if is_identifier(lvalue_str) {
            let env = ctx
                .resolve_binding_env(lvalue_str)
                .unwrap_or_else(|| ctx.current_env());
            let old_val = js_get_property_str(ctx, env, lvalue_str);
            let n = js_to_number(ctx, old_val).ok()?;
            let new_val = if is_inc {
                number_to_value(ctx, n + 1.0)
            } else {
                number_to_value(ctx, n - 1.0)
            };
            js_set_property_str(ctx, env, lvalue_str, new_val);
            return Some(old_val); // postfix returns old value
        }
    }
    // Check for prefix ++ or --
    if s.starts_with("++") || s.starts_with("--") {
        let lvalue_str = s[2..].trim();
        let is_inc = s.starts_with("++");
        // Try property access: ++obj.prop or ++obj[idx]
        if let Some(result) = eval_property_inc_dec(ctx, lvalue_str, is_inc, true) {
            return Some(result);
        }
        // Simple variable
        if is_identifier(lvalue_str) {
            let env = ctx
                .resolve_binding_env(lvalue_str)
                .unwrap_or_else(|| ctx.current_env());
            let old_val = js_get_property_str(ctx, env, lvalue_str);
            let n = js_to_number(ctx, old_val).ok()?;
            let new_val = if is_inc {
                number_to_value(ctx, n + 1.0)
            } else {
                number_to_value(ctx, n - 1.0)
            };
            js_set_property_str(ctx, env, lvalue_str, new_val);
            return Some(new_val); // prefix returns new value
        }
    }
    // Check for typeof operator
    if s.starts_with("typeof ") {
        let operand = s[7..].trim();
        let val = eval_expr(ctx, operand)?;

        // Check for builtin function markers (these are strings that represent constructor functions)
        if js_is_string(ctx, val) != 0 {
            if let Some(bytes) = ctx.string_bytes(val) {
                if let Ok(str_val) = core::str::from_utf8(bytes) {
                    // Constructor functions that should return "function"
                    if str_val == "__builtin_Object__" ||
                       str_val == "__builtin_Array__" ||
                       str_val == "__builtin_String__" ||
                       str_val == "__builtin_Number__" ||
                       str_val == "__builtin_Date__" ||
                       str_val == "__builtin_RegExp__" ||
                       str_val == "__builtin_Function__" ||
                       str_val == "__builtin_Error__" ||
                       str_val == "__builtin_TypeError__" ||
                       str_val == "__builtin_ReferenceError__" ||
                       str_val == "__builtin_SyntaxError__" ||
                       str_val == "__builtin_RangeError__" ||
                       str_val == "__builtin_parseInt__" ||
                       str_val == "__builtin_parseFloat__" ||
                       str_val == "__builtin_eval__" ||
                       str_val == "__builtin_isNaN__" ||
                       str_val == "__builtin_isFinite__" {
                        return Some(js_new_string(ctx, "function"));
                    }
                    // Objects that should return "object"
                    if str_val == "__builtin_Math__" ||
                       str_val == "__builtin_JSON__" ||
                       str_val == "__builtin_console__" {
                        return Some(js_new_string(ctx, "object"));
                    }
                }
            }
        }

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
    // Handle `delete` operator
    if s.starts_with("delete ") {
        let operand = s[7..].trim();
        // Parse the property access: delete obj.prop or delete obj[idx]
        if let Some((obj_expr, prop_name)) = parse_member_access(operand) {
            let obj = eval_expr(ctx, obj_expr)?;
            let deleted = ctx.delete_property_str(obj, prop_name.as_bytes());
            return Some(Value::new_bool(deleted));
        }
        // Try parsing as obj["prop"] or obj[idx]
        if let Some(bracket_start) = operand.rfind('[') {
            if operand.ends_with(']') {
                let obj_expr = &operand[..bracket_start];
                let idx_expr = &operand[bracket_start + 1..operand.len() - 1];
                let obj = eval_expr(ctx, obj_expr)?;
                let idx_val = eval_expr(ctx, idx_expr)?;
                // Extract string to avoid borrow issues
                let name_str: Option<String> = ctx.string_bytes(idx_val)
                    .and_then(|bytes| core::str::from_utf8(bytes).ok())
                    .map(|s| s.to_string());
                if let Some(name) = name_str {
                    let deleted = ctx.delete_property_str(obj, name.as_bytes());
                    return Some(Value::new_bool(deleted));
                }
                if let Some(n) = idx_val.int32() {
                    let deleted = ctx.delete_property_index(obj, n as u32);
                    return Some(Value::new_bool(deleted));
                }
            }
        }
        // Deleting a variable returns true but does nothing
        return Some(Value::TRUE);
    }
    // Handle `new` keyword for constructor calls
    if s.starts_with("new ") {
        let ctor_expr = s[4..].trim();
        // Parse the constructor call: "new Foo(args)" or "new Foo"
        // Find where arguments start (if any)
        let (ctor_name, args_str) = if let Some(paren_start) = ctor_expr.find('(') {
            let (name_part, rest) = ctor_expr.split_at(paren_start);
            let (inside, _) = extract_paren(rest)?;
            (name_part.trim(), Some(inside))
        } else {
            (ctor_expr, None)
        };

        // Evaluate the constructor
        let ctor_val = eval_expr(ctx, ctor_name)?;

        // Parse arguments
        let mut args = Vec::new();
        if let Some(args_src) = args_str {
            let arg_list = split_top_level(args_src)?;
            for arg in arg_list {
                let arg_trim = arg.trim();
                if arg_trim.is_empty() {
                    continue;
                }
                args.push(eval_expr(ctx, arg_trim)?);
            }
        }

        // Check if it's a closure (user-defined function)
        let closure_marker = js_get_property_str(ctx, ctor_val, "__closure__");
        if closure_marker == Value::TRUE {
            // Create a new object instance
            let new_obj = js_new_object(ctx);

            // Set up the prototype chain
            let ctor_proto = js_get_property_str(ctx, ctor_val, "prototype");
            if !ctor_proto.is_undefined() && !ctor_proto.is_null() {
                js_set_property_str(ctx, new_obj, "__proto__", ctor_proto);
            }

            // Call the constructor with `this` bound to the new object
            let result = call_closure_with_this(ctx, ctor_val, new_obj, &args);

            // If the constructor explicitly returns an object, return that; otherwise return new_obj
            if let Some(ret_val) = result {
                if ret_val.is_ptr() && !ret_val.is_null() && !ret_val.is_undefined() {
                    return Some(ret_val);
                }
            }
            return Some(new_obj);
        }

        // Check if it's a builtin marker
        if let Some(bytes) = ctx.string_bytes(ctor_val) {
            if let Ok(marker) = core::str::from_utf8(bytes) {
                match marker {
                    "__builtin_Object__" => {
                        return Some(js_new_object(ctx));
                    }
                    "__builtin_ArrayBuffer__" => {
                        let len = if !args.is_empty() {
                            js_to_int32(ctx, args[0]).unwrap_or(0).max(0) as usize
                        } else {
                            0
                        };
                        return Some(create_arraybuffer(ctx, len));
                    }
                    "__builtin_Uint8Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Uint8Array, &args));
                    }
                    "__builtin_Uint8ClampedArray__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Uint8cArray, &args));
                    }
                    "__builtin_Int8Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Int8Array, &args));
                    }
                    "__builtin_Int16Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Int16Array, &args));
                    }
                    "__builtin_Uint16Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Uint16Array, &args));
                    }
                    "__builtin_Int32Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Int32Array, &args));
                    }
                    "__builtin_Uint32Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Uint32Array, &args));
                    }
                    "__builtin_Float32Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Float32Array, &args));
                    }
                    "__builtin_Float64Array__" => {
                        return Some(build_typed_array_from_args(ctx, JSObjectClassEnum::Float64Array, &args));
                    }
                    "__builtin_Array__" => {
                        if args.len() == 1 {
                            if let Some(len) = args[0].int32() {
                                return Some(js_new_array(ctx, len));
                            }
                        }
                        // Create array with elements
                        let arr = js_new_array(ctx, args.len() as i32);
                        for (i, arg) in args.iter().enumerate() {
                            js_set_property_uint32(ctx, arr, i as u32, *arg);
                        }
                        return Some(arr);
                    }
                    "__builtin_Error__" => {
                        let msg = if !args.is_empty() {
                            if let Some(bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(bytes).unwrap_or("").to_string()
                            } else {
                                "".to_string()
                            }
                        } else {
                            "".to_string()
                        };
                        return Some(js_throw_error(ctx, JSObjectClassEnum::Error, &msg));
                    }
                    "__builtin_Function__" => {
                        // new Function(params..., body)
                        if !args.is_empty() {
                            // Extract body string first to avoid borrow issues
                            let body = {
                                let body_bytes = ctx.string_bytes(args[args.len() - 1]);
                                body_bytes.and_then(|b| core::str::from_utf8(b).ok()).unwrap_or("").to_string()
                            };
                            // Extract parameter strings
                            let mut params = Vec::new();
                            for i in 0..args.len() - 1 {
                                if let Some(bytes) = ctx.string_bytes(args[i]) {
                                    if let Ok(s) = core::str::from_utf8(bytes) {
                                        params.push(s.to_string());
                                    }
                                }
                            }
                            return create_function(ctx, &params, &body);
                        }
                        return create_function(ctx, &[], "");
                    }
                    _ => {}
                }
            }
        }

        // For unrecognized constructors, return undefined
        return Some(Value::UNDEFINED);
    }
    if s.starts_with("function") {
        if let Some(val) = eval_value(ctx, s) {
            return Some(val);
        }
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
            let mut arg_list = split_top_level(inside)?;
            if arg_list.is_empty() && !inside.trim().is_empty() {
                arg_list.push(inside.trim());
            }
            let mut args = Vec::new();
            for arg in arg_list {
                let arg_trim = arg.trim();
                if arg_trim.is_empty() {
                    continue;
                }
                let v = eval_expr(ctx, arg_trim)?;
                if v.is_undefined() && is_identifier(arg_trim) {
                    if let Some((_, gv)) = ctx.resolve_binding(arg_trim) {
                        args.push(gv);
                        continue;
                    }
                }
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
                    if marker == "__builtin_array_push__" {
                        for arg in &args {
                            js_array_push(ctx, this_val, *arg);
                        }
                        val = js_get_property_str(ctx, this_val, "length");
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_pop__" {
                        val = js_array_pop(ctx, this_val);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_charAt__" {
                        if args.len() == 1 {
                            if let Some(idx) = args[0].int32() {
                                if let Some(units) = string_utf16_units(ctx, this_val) {
                                    if idx >= 0 && (idx as usize) < units.len() {
                                        let unit = units[idx as usize];
                                        let s = String::from_utf16_lossy(&[unit]);
                                        val = js_new_string(ctx, &s);
                                    } else {
                                        val = js_new_string(ctx, "");
                                    }
                                    this_val = Value::UNDEFINED;
                                    rest = next;
                                    continue;
                                }
                            }
                        }
                    } else if marker == "__builtin_string_concat__" {
                        // Ported from mquickjs.c:13489-13510 js_string_concat
                        // Get base string
                        let mut result = if let Some(str_bytes) = ctx.string_bytes(this_val) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                s.to_string()
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };
                        
                        // Concatenate all arguments
                        for arg in &args {
                            if let Some(arg_bytes) = ctx.string_bytes(*arg) {
                                if let Ok(s) = core::str::from_utf8(arg_bytes) {
                                    result.push_str(s);
                                }
                            } else if let Some(n) = arg.int32() {
                                result.push_str(&n.to_string());
                            } else if *arg == Value::TRUE {
                                result.push_str("true");
                            } else if *arg == Value::FALSE {
                                result.push_str("false");
                            }
                        }
                        
                        val = js_new_string(ctx, &result);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_substring__" {
                        if args.len() >= 1 && args.len() <= 2 {
                            if let Some(units) = string_utf16_units(ctx, this_val) {
                                let len = units.len() as i32;
                                let mut start = args[0].int32().unwrap_or(0).max(0).min(len);
                                let mut end = if args.len() == 2 {
                                    args[1].int32().unwrap_or(len).max(0).min(len)
                                } else {
                                    len
                                };
                                if start > end {
                                    core::mem::swap(&mut start, &mut end);
                                }
                                let s = String::from_utf16_lossy(&units[start as usize..end as usize]);
                                val = js_new_string(ctx, &s);
                                this_val = Value::UNDEFINED;
                                rest = next;
                                continue;
                            }
                        }
                        val = js_new_string(ctx, "");
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_substr__" {
                        if args.len() >= 1 {
                            if let Some(units) = string_utf16_units(ctx, this_val) {
                                let len = units.len() as i32;
                                let mut start = args[0].int32().unwrap_or(0);
                                if start < 0 {
                                    start = (len + start).max(0);
                                } else if start > len {
                                    start = len;
                                }
                                let count = if args.len() >= 2 {
                                    args[1].int32().unwrap_or(0).max(0)
                                } else {
                                    len - start
                                };
                                let end = (start + count).min(len);
                                let s = String::from_utf16_lossy(&units[start as usize..end as usize]);
                                val = js_new_string(ctx, &s);
                                this_val = Value::UNDEFINED;
                                rest = next;
                                continue;
                            }
                        }
                        val = js_new_string(ctx, "");
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
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
                    } else if marker == "__builtin_string_lastIndexOf__" {
                        if args.len() == 1 {
                            if let Some(needle_bytes) = ctx.string_bytes(args[0]) {
                                if let Some(haystack_bytes) = ctx.string_bytes(this_val) {
                                    // Search from end to find last occurrence
                                    let needle = needle_bytes;
                                    let haystack = haystack_bytes;
                                    if needle.is_empty() {
                                        val = Value::from_int32(haystack.len() as i32);
                                    } else {
                                        let mut found = -1;
                                        // Search backwards
                                        if haystack.len() >= needle.len() {
                                            for i in (0..=(haystack.len() - needle.len())).rev() {
                                                if &haystack[i..i + needle.len()] == needle {
                                                    found = i as i32;
                                                    break;
                                                }
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
                        if let Some(units) = string_utf16_units(ctx, this_val) {
                            let len = units.len() as i32;
                            let mut start = if args.len() >= 1 { args[0].int32().unwrap_or(0) } else { 0 };
                            let mut end = if args.len() >= 2 { args[1].int32().unwrap_or(len) } else { len };
                            if start < 0 {
                                start = (len + start).max(0);
                            } else {
                                start = start.min(len);
                            }
                            if end < 0 {
                                end = (len + end).max(0);
                            } else {
                                end = end.min(len);
                            }
                            if end < start {
                                end = start;
                            }
                            let s = String::from_utf16_lossy(&units[start as usize..end as usize]);
                            val = js_new_string(ctx, &s);
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
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
                        // Get array length
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        if let Some(len) = len_val.int32() {
                            let count = args.len() as i32;
                            if count == 0 {
                                val = Value::from_int32(len);
                            } else {
                                // Shift all elements up by count
                                for i in (0..len).rev() {
                                    let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                    js_set_property_uint32(ctx, this_val, (i + count) as u32, elem);
                                }
                                // Insert new elements at start
                                for (i, arg) in args.iter().enumerate() {
                                    js_set_property_uint32(ctx, this_val, i as u32, *arg);
                                }
                                // Set new length
                                js_set_property_str(ctx, this_val, "length", Value::from_int32(len + count));
                                val = Value::from_int32(len + count);
                            }
                        } else {
                            val = Value::UNDEFINED;
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
                    } else if marker == "__builtin_typedarray_toString__" {
                        let data_ptr = get_typedarray_data(ctx, this_val);
                        if let Some(ptr) = data_ptr {
                            let data = unsafe { &*ptr };
                            let mut out = String::new();
                            for i in 0..data.length {
                                if i > 0 {
                                    out.push(',');
                                }
                                let v = typed_array_get_element(ctx, this_val, i as u32);
                                let s_val = js_to_string(ctx, v);
                                if let Some(bytes) = ctx.string_bytes(s_val) {
                                    out.push_str(core::str::from_utf8(bytes).unwrap_or(""));
                                }
                            }
                            val = js_new_string(ctx, &out);
                        } else {
                            val = js_new_string(ctx, "");
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_typedarray_set__" {
                        if args.len() >= 1 {
                            let offset = if args.len() >= 2 {
                                js_to_int32(ctx, args[1]).unwrap_or(0).max(0) as usize
                            } else {
                                0
                            };
                            let data_ptr = get_typedarray_data(ctx, this_val);
                            if let Some(ptr) = data_ptr {
                                let data = unsafe { &*ptr };
                                let len = data.length;
                                let mut idx = 0usize;
                                if let Some(src_class) = ctx.object_class_id(args[0]) {
                                    if typed_array_kind_from_class_id(src_class).is_some() {
                                        let src_len = js_get_property_str(ctx, args[0], "length").int32().unwrap_or(0).max(0) as usize;
                                        while idx < src_len && offset + idx < len {
                                            let v = typed_array_get_element(ctx, args[0], idx as u32);
                                            typed_array_set_element(ctx, this_val, (offset + idx) as u32, v);
                                            idx += 1;
                                        }
                                    } else if src_class == JSObjectClassEnum::Array as u32 {
                                        let src_len = js_get_property_str(ctx, args[0], "length").int32().unwrap_or(0).max(0) as usize;
                                        while idx < src_len && offset + idx < len {
                                            let v = js_get_property_uint32(ctx, args[0], idx as u32);
                                            typed_array_set_element(ctx, this_val, (offset + idx) as u32, v);
                                            idx += 1;
                                        }
                                    }
                                }
                            }
                        }
                        val = Value::UNDEFINED;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_typedarray_subarray__" {
                        let data_ptr = get_typedarray_data(ctx, this_val);
                        if let Some(ptr) = data_ptr {
                            let data = unsafe { &*ptr };
                            let len = data.length as i32;
                            let mut begin = if args.len() >= 1 { js_to_int32(ctx, args[0]).unwrap_or(0) } else { 0 };
                            let mut end = if args.len() >= 2 { js_to_int32(ctx, args[1]).unwrap_or(len) } else { len };
                            if begin < 0 { begin = (len + begin).max(0); }
                            if end < 0 { end = (len + end).max(0); }
                            if begin > len { begin = len; }
                            if end > len { end = len; }
                            if end < begin { end = begin; }
                            let new_len = (end - begin) as usize;
                            let class_id = ctx.object_class_id(this_val).unwrap_or(JSObjectClassEnum::TypedArray as u32);
                            let offset = data.offset + (begin as usize) * data.kind.elem_size();
                            if let Some(class_enum) = typed_array_class_enum_from_id(class_id) {
                                val = create_typed_array(ctx, class_enum, data.buffer, offset, new_len);
                            } else {
                                val = Value::UNDEFINED;
                            }
                        } else {
                            val = Value::UNDEFINED;
                        }
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
                    } else if marker == "__builtin_string_toLocaleUpperCase__" {
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
                    } else if marker == "__builtin_string_toLocaleLowerCase__" {
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
                    } else if marker == "__builtin_Math_sin__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.sin());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_cos__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.cos());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_tan__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.tan());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_asin__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.asin());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_acos__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.acos());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_atan__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.atan());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_atan2__" {
                        if args.len() == 2 {
                            let y = js_to_number(ctx, args[0]).ok()?;
                            let x = js_to_number(ctx, args[1]).ok()?;
                            val = number_to_value(ctx, y.atan2(x));
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_exp__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.exp());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_log__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            val = number_to_value(ctx, n.ln());
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_log2__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            let denom = core::f64::consts::LN_2;
                            val = number_to_value(ctx, n.ln() / denom);
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_log10__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            let denom = core::f64::consts::LN_10;
                            val = number_to_value(ctx, n.ln() / denom);
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_fround__" {
                        if args.len() == 1 {
                            let n = js_to_number(ctx, args[0]).ok()?;
                            let f = n as f32;
                            val = number_to_value(ctx, f as f64);
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_imul__" {
                        if args.len() == 2 {
                            let a = js_to_int32(ctx, args[0]).unwrap_or(0);
                            let b = js_to_int32(ctx, args[1]).unwrap_or(0);
                            val = Value::from_int32(a.wrapping_mul(b));
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Math_clz32__" {
                        if args.len() == 1 {
                            let n = js_to_uint32(ctx, args[0]).unwrap_or(0);
                            let count = n.leading_zeros() as i32;
                            val = Value::from_int32(count);
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
                    } else if marker == "__builtin_string_match__" {
                        if args.is_empty() {
                            val = Value::NULL;
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                        let input_val = coerce_to_string_value(ctx, this_val);
                        let s = value_to_string(ctx, input_val);
                        let (pattern, flags) = if let Some((src, flg)) = regexp_parts(ctx, args[0]) {
                            (src, flg)
                        } else {
                            (value_to_string(ctx, args[0]), String::new())
                        };
                        let (re, global) = match compile_regex(ctx, &pattern, &flags) {
                            Ok(v) => v,
                            Err(_) => return None,
                        };
                        if global {
                            let mut matches = Vec::new();
                            for m in re.find_iter(&s) {
                                match m {
                                    Ok(mm) => matches.push(mm.as_str().to_string()),
                                    Err(_) => return None,
                                }
                            }
                            if matches.is_empty() {
                                val = Value::NULL;
                            } else {
                                let arr = js_new_array(ctx, matches.len() as i32);
                                for (i, m) in matches.iter().enumerate() {
                                    let mv = js_new_string(ctx, m);
                                    js_set_property_uint32(ctx, arr, i as u32, mv);
                                }
                                val = arr;
                            }
                        } else if let Ok(Some(caps)) = re.captures(&s) {
                            let arr = js_new_array(ctx, caps.len() as i32);
                            for i in 0..caps.len() {
                                if let Some(m) = caps.get(i) {
                                    let mv = js_new_string(ctx, m.as_str());
                                    js_set_property_uint32(ctx, arr, i as u32, mv);
                                } else {
                                    js_set_property_uint32(ctx, arr, i as u32, Value::UNDEFINED);
                                }
                            }
                            let idx = caps.get(0).map(|m| m.start() as i32).unwrap_or(0);
                            let _ = js_set_property_str(ctx, arr, "index", Value::from_int32(idx));
                            let _ = js_set_property_str(ctx, arr, "input", input_val);
                            val = arr;
                        } else {
                            val = Value::NULL;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_matchAll__" {
                        let input_val = coerce_to_string_value(ctx, this_val);
                        let s = value_to_string(ctx, input_val);
                        let (pattern, flags, global_required) = if args.is_empty() {
                            (String::new(), "g".to_string(), true)
                        } else if let Some((src, flg)) = regexp_parts(ctx, args[0]) {
                            (src, flg, true)
                        } else {
                            (value_to_string(ctx, args[0]), "g".to_string(), true)
                        };
                        let (re, global) = match compile_regex(ctx, &pattern, &flags) {
                            Ok(v) => v,
                            Err(_) => return None,
                        };
                        if global_required && !global {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "matchAll requires a global RegExp");
                            return None;
                        }
                        let mut matches = Vec::new();
                        for m in re.find_iter(&s) {
                            match m {
                                Ok(mm) => matches.push(mm.as_str().to_string()),
                                Err(_) => return None,
                            }
                        }
                        let arr = js_new_array(ctx, matches.len() as i32);
                        for (i, m) in matches.iter().enumerate() {
                            let mv = js_new_string(ctx, m);
                            js_set_property_uint32(ctx, arr, i as u32, mv);
                        }
                        val = arr;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_search__" {
                        if args.is_empty() {
                            val = Value::from_int32(-1);
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                        let s = value_to_string(ctx, this_val);
                        let (pattern, flags) = if let Some((src, flg)) = regexp_parts(ctx, args[0]) {
                            (src, flg)
                        } else {
                            (value_to_string(ctx, args[0]), String::new())
                        };
                        let (re, _) = match compile_regex(ctx, &pattern, &flags) {
                            Ok(v) => v,
                            Err(_) => return None,
                        };
                        match re.find(&s) {
                            Ok(Some(m)) => val = Value::from_int32(m.start() as i32),
                            Ok(None) => val = Value::from_int32(-1),
                            Err(_) => return None,
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_regexp_test__" {
                        let input = if args.is_empty() {
                            String::new()
                        } else {
                            value_to_string(ctx, args[0])
                        };
                        let (pattern, flags) = regexp_parts(ctx, this_val).unwrap_or_default();
                        let (re, _) = match compile_regex(ctx, &pattern, &flags) {
                            Ok(v) => v,
                            Err(_) => return None,
                        };
                        val = Value::new_bool(re.is_match(&input).unwrap_or(false));
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_regexp_exec__" {
                        let input_val = if args.is_empty() {
                            js_new_string(ctx, "")
                        } else {
                            coerce_to_string_value(ctx, args[0])
                        };
                        let input = value_to_string(ctx, input_val);
                        let (pattern, flags) = regexp_parts(ctx, this_val).unwrap_or_default();
                        let (re, _) = match compile_regex(ctx, &pattern, &flags) {
                            Ok(v) => v,
                            Err(_) => return None,
                        };
                        if let Ok(Some(caps)) = re.captures(&input) {
                            let arr = js_new_array(ctx, caps.len() as i32);
                            for i in 0..caps.len() {
                                if let Some(m) = caps.get(i) {
                                    let mv = js_new_string(ctx, m.as_str());
                                    js_set_property_uint32(ctx, arr, i as u32, mv);
                                } else {
                                    js_set_property_uint32(ctx, arr, i as u32, Value::UNDEFINED);
                                }
                            }
                            let idx = caps.get(0).map(|m| m.start() as i32).unwrap_or(0);
                            let _ = js_set_property_str(ctx, arr, "index", Value::from_int32(idx));
                            let _ = js_set_property_str(ctx, arr, "input", input_val);
                            val = arr;
                        } else {
                            val = Value::NULL;
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
                    } else if marker == "__builtin_array_at__" {
                        // ES2022 feature - Array.at() with negative index support
                        if args.len() > 0 {
                            if let Some(index) = args[0].int32() {
                                let len_val = js_get_property_str(ctx, this_val, "length");
                                let len = len_val.int32().unwrap_or(0);
                                
                                // Handle negative indices (count from end)
                                let actual_index = if index < 0 {
                                    len + index
                                } else {
                                    index
                                };
                                
                                // Check bounds
                                if actual_index >= 0 && actual_index < len {
                                    val = js_get_property_uint32(ctx, this_val, actual_index as u32);
                                } else {
                                    val = Value::UNDEFINED;
                                }
                            } else {
                                val = Value::UNDEFINED;
                            }
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_splice__" {
                        // Ported from mquickjs.c:14478-14548 js_array_splice
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0);

                        // Get start index
                        let start = if args.len() > 0 {
                            if let Some(s) = args[0].int32() {
                                if s < 0 {
                                    (len + s).max(0)
                                } else {
                                    s.min(len)
                                }
                            } else {
                                0
                            }
                        } else {
                            0
                        };

                        // Get delete count
                        let del_count = if args.len() > 1 {
                            if let Some(d) = args[1].int32() {
                                d.max(0).min(len - start)
                            } else {
                                len - start
                            }
                        } else if args.len() == 1 {
                            len - start
                        } else {
                            0
                        };

                        // Items to insert
                        let items: Vec<JSValue> = if args.len() > 2 {
                            args[2..].to_vec()
                        } else {
                            Vec::new()
                        };
                        let item_count = items.len() as i32;

                        // Create result array with deleted elements
                        let result = js_new_array(ctx, del_count);
                        for i in 0..del_count {
                            let elem = js_get_property_uint32(ctx, this_val, (start + i) as u32);
                            js_set_property_uint32(ctx, result, i as u32, elem);
                        }

                        let new_len = len + item_count - del_count;

                        // Shift elements if needed
                        if item_count != del_count {
                            if item_count < del_count {
                                // Shrinking - shift left, then truncate
                                for i in (start + item_count)..new_len {
                                    let src_idx = i + (del_count - item_count);
                                    let elem = js_get_property_uint32(ctx, this_val, src_idx as u32);
                                    js_set_property_uint32(ctx, this_val, i as u32, elem);
                                }
                            } else {
                                // Growing - first expand array by pushing, then shift right
                                let extra = item_count - del_count;
                                for _ in 0..extra {
                                    js_array_push(ctx, this_val, Value::UNDEFINED);
                                }
                                // Now shift elements from right to left (in reverse order)
                                for i in ((start + item_count)..new_len).rev() {
                                    let src_idx = i - extra;
                                    let elem = js_get_property_uint32(ctx, this_val, src_idx as u32);
                                    js_set_property_uint32(ctx, this_val, i as u32, elem);
                                }
                            }
                        }

                        // Insert new items
                        for (i, item) in items.into_iter().enumerate() {
                            js_set_property_uint32(ctx, this_val, (start + i as i32) as u32, item);
                        }

                        // Update length (for shrinking case)
                        js_set_property_str(ctx, this_val, "length", Value::from_int32(new_len));

                        val = result;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_indexOf__" {
                        if args.len() >= 1 {
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0);
                            let search_val = args[0];
                            let mut start = if args.len() >= 2 {
                                js_to_int32(ctx, args[1]).unwrap_or(0)
                            } else {
                                0
                            };
                            if start < 0 {
                                start = (len + start).max(0);
                            }
                            let mut found_idx = -1;
                            if start < len {
                                for i in start..len {
                                    let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                    if elem.0 == search_val.0 {
                                        found_idx = i;
                                        break;
                                    }
                                }
                            }
                            val = Value::from_int32(found_idx);
                        } else {
                            val = Value::from_int32(-1);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_lastIndexOf__" {
                        if args.len() >= 1 {
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0);
                            let search_val = args[0];
                            let mut start = if args.len() >= 2 {
                                js_to_int32(ctx, args[1]).unwrap_or(len - 1)
                            } else {
                                len - 1
                            };
                            if start >= len {
                                start = len - 1;
                            }
                            if start < 0 {
                                start = len + start;
                            }
                            let mut found_idx = -1;
                            if start >= 0 {
                                let mut i = start;
                                loop {
                                    let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                    if elem.0 == search_val.0 {
                                        found_idx = i;
                                        break;
                                    }
                                    if i == 0 { break; }
                                    i -= 1;
                                }
                            }
                            val = Value::from_int32(found_idx);
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
                    } else if marker == "__builtin_array_forEach__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                call_closure(ctx, callback, &call_args);
                            }
                            val = Value::UNDEFINED;
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_map__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let result = js_new_array(ctx, len as i32);
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                if let Some(mapped) = call_closure(ctx, callback, &call_args) {
                                    js_set_property_uint32(ctx, result, i, mapped);
                                }
                            }
                            val = result;
                        } else {
                            val = js_new_array(ctx, 0);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_filter__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let result = js_new_array(ctx, 0);
                            let mut result_idx = 0u32;
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                if let Some(res) = call_closure(ctx, callback, &call_args) {
                                    if is_truthy(ctx, res) {
                                        js_set_property_uint32(ctx, result, result_idx, elem);
                                        result_idx += 1;
                                    }
                                }
                            }
                            val = result;
                        } else {
                            val = js_new_array(ctx, 0);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_reduce__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let mut accumulator = if args.len() >= 2 {
                                args[1]
                            } else if len > 0 {
                                js_get_property_uint32(ctx, this_val, 0)
                            } else {
                                Value::UNDEFINED
                            };
                            let start_idx = if args.len() >= 2 { 0 } else { 1 };
                            for i in start_idx..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [accumulator, elem, idx_val, this_val];
                                if let Some(res) = call_closure(ctx, callback, &call_args) {
                                    accumulator = res;
                                }
                            }
                            val = accumulator;
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_reduceRight__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as i32;
                            if len <= 0 {
                                val = Value::UNDEFINED;
                                this_val = Value::UNDEFINED;
                                rest = next;
                                continue;
                            }
                            let mut accumulator = if args.len() >= 2 {
                                args[1]
                            } else {
                                js_get_property_uint32(ctx, this_val, (len - 1) as u32)
                            };
                            let mut i = if args.len() >= 2 { len - 1 } else { len - 2 };
                            while i >= 0 {
                                let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [accumulator, elem, idx_val, this_val];
                                if let Some(res) = call_closure(ctx, callback, &call_args) {
                                    accumulator = res;
                                }
                                if i == 0 { break; }
                                i -= 1;
                            }
                            val = accumulator;
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_every__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let mut all_true = true;
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                if let Some(res) = call_closure(ctx, callback, &call_args) {
                                    if !is_truthy(ctx, res) {
                                        all_true = false;
                                        break;
                                    }
                                }
                            }
                            val = Value::new_bool(all_true);
                        } else {
                            val = Value::TRUE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_some__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let mut any_true = false;
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                if let Some(res) = call_closure(ctx, callback, &call_args) {
                                    if is_truthy(ctx, res) {
                                        any_true = true;
                                        break;
                                    }
                                }
                            }
                            val = Value::new_bool(any_true);
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_find__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let mut found_elem = Value::UNDEFINED;
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                if let Some(res) = call_closure(ctx, callback, &call_args) {
                                    if is_truthy(ctx, res) {
                                        found_elem = elem;
                                        break;
                                    }
                                }
                            }
                            val = found_elem;
                        } else {
                            val = Value::UNDEFINED;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_findIndex__" {
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let mut found_idx = -1i32;
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                if let Some(res) = call_closure(ctx, callback, &call_args) {
                                    if is_truthy(ctx, res) {
                                        found_idx = i as i32;
                                        break;
                                    }
                                }
                            }
                            val = Value::from_int32(found_idx);
                        } else {
                            val = Value::from_int32(-1);
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
                            let s = value_to_string(ctx, this_val);
                            if let Some((pattern, flags)) = regexp_parts(ctx, args[0]) {
                                let (re, global) = match compile_regex(ctx, &pattern, &flags) {
                                    Ok(v) => v,
                                    Err(_) => return None,
                                };
                                let replacement = value_to_string(ctx, args[1]);
                                let replaced = if global {
                                    re.replace_all(&s, replacement.as_str())
                                } else {
                                    re.replace(&s, replacement.as_str())
                                };
                                val = js_new_string(ctx, &replaced.to_string());
                            } else {
                                let search = value_to_string(ctx, args[0]);
                                let replacement = value_to_string(ctx, args[1]);
                                let result = s.replacen(&search, &replacement, 1);
                                val = js_new_string(ctx, &result);
                            }
                        } else {
                            val = this_val;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_string_replaceAll__" {
                        // ES2021 feature - replaceAll() replaces all occurrences
                        if args.len() >= 2 {
                            let s = value_to_string(ctx, this_val);
                            if let Some((pattern, flags)) = regexp_parts(ctx, args[0]) {
                                let (re, global) = match compile_regex(ctx, &pattern, &flags) {
                                    Ok(v) => v,
                                    Err(_) => return None,
                                };
                                if !global {
                                    js_throw_error(ctx, JSObjectClassEnum::TypeError, "replaceAll requires a global RegExp");
                                    return None;
                                }
                                let replacement = value_to_string(ctx, args[1]);
                                let replaced = re.replace_all(&s, replacement.as_str());
                                val = js_new_string(ctx, &replaced.to_string());
                            } else {
                                let search = value_to_string(ctx, args[0]);
                                let replacement = value_to_string(ctx, args[1]);
                                let result = s.replace(&search, &replacement);
                                val = js_new_string(ctx, &result);
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
                                if let Some(units) = string_utf16_units(ctx, this_val) {
                                    if idx >= 0 && (idx as usize) < units.len() {
                                        val = Value::from_int32(units[idx as usize] as i32);
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
                    } else if marker == "__builtin_string_codePointAt__" {
                        let idx = if args.len() >= 1 {
                            args[0].int32().unwrap_or(0)
                        } else {
                            0
                        };
                        if let Some(units) = string_utf16_units(ctx, this_val) {
                            if idx >= 0 && (idx as usize) < units.len() {
                                let first = units[idx as usize] as u32;
                                if (0xD800..=0xDBFF).contains(&first) && (idx as usize + 1) < units.len() {
                                    let second = units[idx as usize + 1] as u32;
                                    if (0xDC00..=0xDFFF).contains(&second) {
                                        let cp = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                                        val = number_to_value(ctx, cp as f64);
                                    } else {
                                        val = number_to_value(ctx, first as f64);
                                    }
                                } else {
                                    val = number_to_value(ctx, first as f64);
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
                    } else if marker == "__builtin_String__" {
                        if args.is_empty() {
                            val = js_new_string(ctx, "");
                        } else {
                            val = js_to_string(ctx, args[0]);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Number__" {
                        if args.is_empty() {
                            val = Value::from_int32(0);
                        } else {
                            let n = js_to_number(ctx, args[0]).unwrap_or(f64::NAN);
                            val = number_to_value(ctx, n);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Boolean__" {
                        if args.is_empty() {
                            val = Value::FALSE;
                        } else {
                            val = Value::new_bool(crate::evals::is_truthy(ctx, args[0]));
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_parseInt__" {
                        if args.len() >= 1 {
                            let s = if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(str_bytes).unwrap_or("").to_string()
                            } else {
                                let s_val = js_to_string(ctx, args[0]);
                                if let Some(bytes) = ctx.string_bytes(s_val) {
                                    core::str::from_utf8(bytes).unwrap_or("").to_string()
                                } else {
                                    String::new()
                                }
                            };
                            let mut s = s.trim_start();
                            let mut sign = 1.0;
                            if s.starts_with('-') {
                                sign = -1.0;
                                s = &s[1..];
                            } else if s.starts_with('+') {
                                s = &s[1..];
                            }
                            let radix = if args.len() >= 2 {
                                args[1].int32().unwrap_or(0)
                            } else {
                                0
                            };
                            let mut radix = radix;
                            let mut s_ref = s;
                            if radix == 0 {
                                if s_ref.starts_with("0x") || s_ref.starts_with("0X") {
                                    radix = 16;
                                    s_ref = &s_ref[2..];
                                } else {
                                    radix = 10;
                                }
                            } else if radix == 16 && (s_ref.starts_with("0x") || s_ref.starts_with("0X")) {
                                s_ref = &s_ref[2..];
                            }
                            if radix < 2 || radix > 36 {
                                val = number_to_value(ctx, f64::NAN);
                            } else {
                                let mut acc: u64 = 0;
                                let mut any = false;
                                for ch in s_ref.chars() {
                                    if let Some(d) = ch.to_digit(radix as u32) {
                                        acc = acc.saturating_mul(radix as u64).saturating_add(d as u64);
                                        any = true;
                                    } else {
                                        break;
                                    }
                                }
                                if any {
                                    val = number_to_value(ctx, sign * (acc as f64));
                                } else {
                                    val = number_to_value(ctx, f64::NAN);
                                }
                            }
                        } else {
                            val = number_to_value(ctx, f64::NAN);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_eval__" {
                        if args.is_empty() {
                            val = Value::UNDEFINED;
                        } else if let Some(bytes) = ctx.string_bytes(args[0]) {
                            let code = core::str::from_utf8(bytes).unwrap_or("").to_string();
                            val = js_eval(ctx, &code, "<eval>", JS_EVAL_RETVAL);
                        } else {
                            val = args[0];
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_parseFloat__" {
                        if args.len() >= 1 {
                            if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                                if let Ok(s) = core::str::from_utf8(str_bytes) {
                                    let trimmed = s.trim_start();
                                    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                                        val = number_to_value(ctx, 0.0);
                                    } else if let Ok(n) = trimmed.parse::<f64>() {
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
                    } else if marker == "__builtin_parseFloat__" {
                        if args.len() >= 1 {
                            let s = if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(str_bytes).unwrap_or("").to_string()
                            } else {
                                let s_val = js_to_string(ctx, args[0]);
                                if let Some(bytes) = ctx.string_bytes(s_val) {
                                    core::str::from_utf8(bytes).unwrap_or("").to_string()
                                } else {
                                    String::new()
                                }
                            };
                            let s = s.trim_start();
                            if s.starts_with("Infinity") || s.starts_with("+Infinity") {
                                val = number_to_value(ctx, f64::INFINITY);
                            } else if s.starts_with("-Infinity") {
                                val = number_to_value(ctx, f64::NEG_INFINITY);
                            } else {
                                let mut end = 0usize;
                                let mut seen_digit = false;
                                let mut seen_dot = false;
                                let mut seen_exp = false;
                                let chars: Vec<char> = s.chars().collect();
                                for i in 0..chars.len() {
                                    let ch = chars[i];
                                    if ch.is_ascii_digit() {
                                        seen_digit = true;
                                        end = i + 1;
                                        continue;
                                    }
                                    if ch == '.' && !seen_dot && !seen_exp {
                                        seen_dot = true;
                                        end = i + 1;
                                        continue;
                                    }
                                    if (ch == 'e' || ch == 'E') && seen_digit && !seen_exp {
                                        seen_exp = true;
                                        end = i + 1;
                                        continue;
                                    }
                                    if (ch == '+' || ch == '-') && seen_exp {
                                        let prev = if i > 0 { chars[i - 1] } else { '\0' };
                                        if prev == 'e' || prev == 'E' {
                                            end = i + 1;
                                            continue;
                                        }
                                    }
                                    break;
                                }
                                if end == 0 || !seen_digit {
                                    val = number_to_value(ctx, f64::NAN);
                                } else {
                                    let num_str = &s[..end];
                                    val = number_to_value(ctx, num_str.parse::<f64>().unwrap_or(f64::NAN));
                                }
                            }
                        } else {
                            val = number_to_value(ctx, f64::NAN);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_isNaN__" {
                    } else if marker == "__builtin_Date_now__" {
                        val = js_date_now(ctx);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_console_log__" {
                        js_console_log(ctx, &args);
                        val = Value::UNDEFINED;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_RegExp__" {
                        let (pattern, flags) = if args.is_empty() {
                            (String::new(), String::new())
                        } else if let Some((src, flg)) = regexp_parts(ctx, args[0]) {
                            let flags = if args.len() >= 2 && !args[1].is_undefined() {
                                value_to_string(ctx, args[1])
                            } else {
                                flg
                            };
                            (src, flags)
                        } else {
                            let pattern = value_to_string(ctx, args[0]);
                            let flags = if args.len() >= 2 && !args[1].is_undefined() {
                                value_to_string(ctx, args[1])
                            } else {
                                String::new()
                            };
                            (pattern, flags)
                        };
                        val = js_new_regexp(ctx, &pattern, &flags);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Error__" {
                        // Error constructor - create error with message
                        let msg = if args.len() > 0 {
                            if let Some(bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(bytes).unwrap_or("Error").to_string()
                            } else {
                                "Error".to_string()
                            }
                        } else {
                            "Error".to_string()
                        };
                        val = js_new_error_object(ctx, JSObjectClassEnum::Error, "Error", &msg);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_TypeError__" {
                        let msg = if args.len() > 0 {
                            if let Some(bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(bytes).unwrap_or("TypeError").to_string()
                            } else {
                                "TypeError".to_string()
                            }
                        } else {
                            "TypeError".to_string()
                        };
                        val = js_new_error_object(ctx, JSObjectClassEnum::TypeError, "TypeError", &msg);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_ReferenceError__" {
                        let msg = if args.len() > 0 {
                            if let Some(bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(bytes).unwrap_or("ReferenceError").to_string()
                            } else {
                                "ReferenceError".to_string()
                            }
                        } else {
                            "ReferenceError".to_string()
                        };
                        val = js_new_error_object(ctx, JSObjectClassEnum::ReferenceError, "ReferenceError", &msg);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_SyntaxError__" {
                        let msg = if args.len() > 0 {
                            if let Some(bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(bytes).unwrap_or("SyntaxError").to_string()
                            } else {
                                "SyntaxError".to_string()
                            }
                        } else {
                            "SyntaxError".to_string()
                        };
                        val = js_new_error_object(ctx, JSObjectClassEnum::SyntaxError, "SyntaxError", &msg);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_RangeError__" {
                        let msg = if args.len() > 0 {
                            if let Some(bytes) = ctx.string_bytes(args[0]) {
                                core::str::from_utf8(bytes).unwrap_or("RangeError").to_string()
                            } else {
                                "RangeError".to_string()
                            }
                        } else {
                            "RangeError".to_string()
                        };
                        val = js_new_error_object(ctx, JSObjectClassEnum::RangeError, "RangeError", &msg);
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
                    } else if marker == "__builtin_Object_values__" {
                        // ES2017 feature - not in mquickjs but useful
                        // Returns array of object's own enumerable property values
                        if args.len() == 1 {
                            let obj = args[0];
                            
                            // Get keys from the object
                            if let Some(keys) = ctx.object_keys(obj) {
                                // Create array for result
                                let arr = js_new_array(ctx, keys.len() as i32);
                                
                                // Populate array with values
                                for (i, key) in keys.iter().enumerate() {
                                    let value = js_get_property_str(ctx, obj, key);
                                    js_set_property_uint32(ctx, arr, i as u32, value);
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
                    } else if marker == "__builtin_Object_entries__" {
                        // ES2017 feature - not in mquickjs but useful
                        // Returns array of object's own enumerable [key, value] pairs
                        if args.len() == 1 {
                            let obj = args[0];
                            
                            // Get keys from the object
                            if let Some(keys) = ctx.object_keys(obj) {
                                // Create array for result
                                let arr = js_new_array(ctx, keys.len() as i32);
                                
                                // Populate array with [key, value] pairs
                                for (i, key) in keys.iter().enumerate() {
                                    let value = js_get_property_str(ctx, obj, key);
                                    
                                    // Create [key, value] pair as array
                                    let pair = js_new_array(ctx, 2);
                                    let key_str = js_new_string(ctx, key);
                                    js_set_property_uint32(ctx, pair, 0, key_str);
                                    js_set_property_uint32(ctx, pair, 1, value);
                                    
                                    js_set_property_uint32(ctx, arr, i as u32, pair);
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
                    } else if marker == "__builtin_Object_assign__" {
                        // ES2015 feature - copy properties from sources to target
                        // Object.assign(target, ...sources) returns target
                        if args.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Cannot convert undefined or null to object");
                            return None;
                        }
                        
                        let target = args[0];
                        
                        // Copy properties from each source to target
                        for i in 1..args.len() {
                            let source = args[i];
                            
                            // Get all keys from source object
                            if let Some(keys) = ctx.object_keys(source) {
                                for key in keys.iter() {
                                    let value = js_get_property_str(ctx, source, key);
                                    js_set_property_str(ctx, target, key, value);
                                }
                            }
                        }
                        
                        val = target;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_hasOwnProperty__" {
                        let (target, key) = if args.len() >= 2 {
                            (args[0], args[1])
                        } else if args.len() == 1 {
                            (this_val, args[0])
                        } else {
                            val = Value::FALSE;
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        };
                        if ctx.object_class_id(target).is_none() {
                            val = Value::FALSE;
                        } else {
                            let key_val = if ctx.string_bytes(key).is_some() {
                                key
                            } else {
                                js_to_string(ctx, key)
                            };
                            if let Some(bytes) = ctx.string_bytes(key_val) {
                                let key_str = core::str::from_utf8(bytes).unwrap_or("");
                                if let Some(keys) = ctx.object_keys(target) {
                                    val = Value::new_bool(keys.iter().any(|k| k == key_str));
                                } else {
                                    val = Value::FALSE;
                                }
                            } else {
                                val = Value::FALSE;
                            }
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_create__" {
                        // ES5 Object.create(proto) - create new object with specified prototype
                        if args.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.create requires a prototype");
                            return None;
                        }
                        val = js_object_create(ctx, args[0]);
                        if val.is_exception() {
                            return None;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_freeze__" {
                        // ES5 Object.freeze(obj) - prevent modifications to object
                        // For now, just return the object (no actual freezing)
                        // Full implementation would require object property flags
                        if args.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Cannot convert undefined to object");
                            return None;
                        }
                        val = args[0];
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_seal__" {
                        // ES5 Object.seal(obj) - prevent extensions and mark props non-configurable
                        // Not fully supported; return the object as-is.
                        if args.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Cannot convert undefined to object");
                            return None;
                        }
                        val = args[0];
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_isSealed__" {
                        if args.is_empty() {
                            val = Value::TRUE;
                        } else if ctx.object_class_id(args[0]).is_none() {
                            val = Value::TRUE;
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_isFrozen__" {
                        if args.is_empty() {
                            val = Value::TRUE;
                        } else if ctx.object_class_id(args[0]).is_none() {
                            val = Value::TRUE;
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_defineProperty__" {
                        if args.len() < 3 {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.defineProperty requires an object and a property");
                            return None;
                        }
                        let obj = args[0];
                        if ctx.object_class_id(obj).is_none() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.defineProperty called on non-object");
                            return None;
                        }
                        let key_val = if ctx.string_bytes(args[1]).is_some() {
                            args[1]
                        } else {
                            js_to_string(ctx, args[1])
                        };
                        let key_bytes = ctx
                            .string_bytes(key_val)
                            .map(|bytes| bytes.to_vec())
                            .unwrap_or_default();
                        let key_str = core::str::from_utf8(&key_bytes).unwrap_or("");
                        let desc = args[2];
                        if ctx.object_class_id(desc).is_none() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Property descriptor must be an object");
                            return None;
                        }
                        let prop_val = js_get_property_str(ctx, desc, "value");
                        if key_bytes.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Invalid property key");
                            return None;
                        }
                        let res = js_set_property_str(ctx, obj, key_str, prop_val);
                        if res.is_exception() {
                            return None;
                        }
                        val = obj;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_getOwnPropertyDescriptor__" {
                        if args.len() < 2 {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.getOwnPropertyDescriptor requires an object and a property");
                            return None;
                        }
                        let obj = args[0];
                        if ctx.object_class_id(obj).is_none() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.getOwnPropertyDescriptor called on non-object");
                            return None;
                        }
                        let key_val = if ctx.string_bytes(args[1]).is_some() {
                            args[1]
                        } else {
                            js_to_string(ctx, args[1])
                        };
                        let key_bytes = ctx
                            .string_bytes(key_val)
                            .map(|bytes| bytes.to_vec())
                            .unwrap_or_default();
                        if key_bytes.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Invalid property key");
                            return None;
                        }
                        if !ctx.has_property_str(obj, &key_bytes) {
                            val = Value::UNDEFINED;
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                        let key_str = core::str::from_utf8(&key_bytes).unwrap_or("");
                        let prop_val = js_get_property_str(ctx, obj, key_str);
                        let desc = js_new_object(ctx);
                        js_set_property_str(ctx, desc, "value", prop_val);
                        js_set_property_str(ctx, desc, "writable", Value::TRUE);
                        js_set_property_str(ctx, desc, "enumerable", Value::TRUE);
                        js_set_property_str(ctx, desc, "configurable", Value::TRUE);
                        val = desc;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_getPrototypeOf__" {
                        if args.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.getPrototypeOf requires an object");
                            return None;
                        }
                        val = js_object_get_prototype_of(ctx, args[0]);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_JSON_stringify__" {
                        // JSON.stringify(value) - convert value to JSON string
                        if args.is_empty() {
                            val = Value::UNDEFINED;
                        } else {
                            let value = args[0];
                            let json_str = crate::json::json_stringify_value(ctx, value);
                            val = js_new_string(ctx, &json_str);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_JSON_parse__" {
                        // JSON.parse(text) - parse JSON string to value
                        if args.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "Unexpected end of JSON input");
                            return None;
                        }
                        
                        if let Some(json_bytes) = ctx.string_bytes(args[0]) {
                            let json_str = core::str::from_utf8(json_bytes).unwrap_or("").to_string();
                            match parse_json(ctx, &json_str) {
                                Some(parsed_val) => val = parsed_val,
                                None => {
                                    js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "Unexpected token in JSON");
                                    return None;
                                }
                            }
                        } else {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Cannot convert to string");
                            return None;
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
                    } else if marker == "__builtin_Array_from__" {
                        if args.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Array.from requires an array-like object");
                            return None;
                        }
                        let source = args[0];
                        let map_fn = if args.len() >= 2 { Some(args[1]) } else { None };
                        let this_arg = if args.len() >= 3 { args[2] } else { Value::UNDEFINED };
                        let mut is_string = false;
                        let len = if ctx.string_bytes(source).is_some() {
                            is_string = true;
                            string_utf16_len(ctx, source).unwrap_or(0) as i32
                        } else if ctx.object_class_id(source).is_some() {
                            let len_val = js_get_property_str(ctx, source, "length");
                            let len_num = js_to_number(ctx, len_val).unwrap_or(0.0);
                            let len_num = if len_num.is_nan() || len_num <= 0.0 {
                                0.0
                            } else {
                                len_num
                            };
                            len_num.min(i32::MAX as f64) as i32
                        } else {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Array.from requires an array-like object");
                            return None;
                        };
                        let result = js_new_array(ctx, len);
                        let mut map_closure: Option<JSValue> = None;
                        let mut map_cfunc: Option<(i32, JSValue)> = None;
                        let mut map_marker: Option<String> = None;
                        if let Some(cb) = map_fn {
                            let closure_marker = js_get_property_str(ctx, cb, "__closure__");
                            if closure_marker == Value::TRUE {
                                map_closure = Some(cb);
                            } else if let Some((idx, params)) = ctx.c_function_info(cb) {
                                map_cfunc = Some((idx, params));
                            } else if let Some(bytes) = ctx.string_bytes(cb) {
                                if let Ok(marker) = core::str::from_utf8(bytes) {
                                    map_marker = Some(marker.to_string());
                                }
                            }
                            if map_closure.is_none() && map_cfunc.is_none() && map_marker.is_none() {
                                js_throw_error(ctx, JSObjectClassEnum::TypeError, "Array.from map function is not callable");
                                return None;
                            }
                        }
                        for i in 0..(len.max(0) as u32) {
                            let elem = if is_string {
                                let units = string_utf16_units(ctx, source).unwrap_or_default();
                                let unit = units.get(i as usize).copied().unwrap_or(0);
                                let s = String::from_utf16_lossy(&[unit]);
                                js_new_string(ctx, &s)
                            } else {
                                js_get_property_uint32(ctx, source, i)
                            };
                            let mut out = elem;
                            if map_closure.is_some() || map_cfunc.is_some() || map_marker.is_some() {
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_arg];
                                if let Some(cb) = map_closure {
                                    if let Some(mapped) = call_closure(ctx, cb, &call_args) {
                                        out = mapped;
                                    }
                                } else if let Some((idx, params)) = map_cfunc {
                                    out = call_c_function(ctx, idx, params, this_arg, &call_args);
                                } else if let Some(marker) = map_marker.as_deref() {
                                    if let Some(mapped) = call_builtin_global_marker(ctx, marker, &call_args) {
                                        out = mapped;
                                    }
                                }
                            }
                            let _ = js_set_property_uint32(ctx, result, i, out);
                        }
                        val = result;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Array_of__" {
                        let result = js_new_array(ctx, args.len() as i32);
                        for (i, arg) in args.iter().enumerate() {
                            let _ = js_set_property_uint32(ctx, result, i as u32, *arg);
                        }
                        val = result;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_flat__" {
                        // ES2019 Array.flat() - flatten nested arrays
                        // Default depth is 1, can specify different depth
                        let depth = if args.is_empty() { 1 } else {
                            args[0].int32().unwrap_or(1)
                        };
                        
                        let result = js_new_array(ctx, 0);
                        flatten_array(ctx, this_val, result, depth);
                        val = result;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_sort__" {
                        // Array.sort() - sort array in place (numeric default here; comparator supported)
                        if let Some(arr_len_val) = ctx.get_property_str(this_val, b"length") {
                            if let Some(len) = arr_len_val.int32() {
                                let comparator = if args.len() >= 1 { Some(args[0]) } else { None };
                                // Simple stable bubble sort implementation
                                for i in 0..len {
                                    for j in 0..(len - i - 1) {
                                        let a = js_get_property_uint32(ctx, this_val, j as u32);
                                        let b = js_get_property_uint32(ctx, this_val, (j + 1) as u32);
                                        if a.is_undefined() {
                                            if !b.is_undefined() {
                                                js_set_property_uint32(ctx, this_val, j as u32, b);
                                                js_set_property_uint32(ctx, this_val, (j + 1) as u32, a);
                                            }
                                            continue;
                                        }
                                        if b.is_undefined() {
                                            continue;
                                        }
                                        let cmp = if let Some(cb) = comparator {
                                            if let Some(res) = call_closure(ctx, cb, &[a, b]) {
                                                js_to_number(ctx, res).unwrap_or(0.0)
                                            } else {
                                                0.0
                                            }
                                        } else {
                                            let a_num = if let Some(n) = a.int32() {
                                                n as f64
                                            } else if let Ok(n) = js_to_number(ctx, a) {
                                                n
                                            } else {
                                                0.0
                                            };
                                            let b_num = if let Some(n) = b.int32() {
                                                n as f64
                                            } else if let Ok(n) = js_to_number(ctx, b) {
                                                n
                                            } else {
                                                0.0
                                            };
                                            a_num - b_num
                                        };
                                        if cmp > 0.0 {
                                            js_set_property_uint32(ctx, this_val, j as u32, b);
                                            js_set_property_uint32(ctx, this_val, (j + 1) as u32, a);
                                        }
                                    }
                                }
                            }
                        }
                        val = this_val;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_array_flatMap__" {
                        // ES2019 Array.flatMap() - map then flatten by 1 level
                        if args.len() >= 1 {
                            let callback = args[0];
                            let len_val = js_get_property_str(ctx, this_val, "length");
                            let len = len_val.int32().unwrap_or(0) as u32;
                            let result = js_new_array(ctx, 0);
                            for i in 0..len {
                                let elem = js_get_property_uint32(ctx, this_val, i);
                                let idx_val = Value::from_int32(i as i32);
                                let call_args = [elem, idx_val, this_val];
                                let mapped = call_closure(ctx, callback, &call_args).unwrap_or(Value::UNDEFINED);
                                if let Some(class_id) = ctx.object_class_id(mapped) {
                                    if class_id == JSObjectClassEnum::Array as u32 {
                                        let mlen_val = js_get_property_str(ctx, mapped, "length");
                                        let mlen = mlen_val.int32().unwrap_or(0) as u32;
                                        for j in 0..mlen {
                                            let mv = js_get_property_uint32(ctx, mapped, j);
                                            let _ = js_array_push(ctx, result, mv);
                                        }
                                        continue;
                                    }
                                }
                                let _ = js_array_push(ctx, result, mapped);
                            }
                            val = result;
                        } else {
                            val = js_new_array(ctx, 0);
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Number_isInteger__" {
                        if args.len() == 1 {
                            if args[0].is_number() {
                                val = Value::TRUE;
                            } else if let Some(f) = ctx.float_value(args[0]) {
                                val = Value::new_bool(f.is_finite() && f.fract() == 0.0);
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Number_isNaN__" {
                        // ES2015 Number.isNaN() - check if value is NaN without coercion
                        // More robust than global isNaN() which coerces to number first
                        if args.len() == 1 {
                            if args[0].is_number() {
                                val = Value::FALSE;
                            } else if let Some(f) = ctx.float_value(args[0]) {
                                val = Value::new_bool(f.is_nan());
                            } else {
                                // If not a number at all, return false (unlike global isNaN)
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Number_isFinite__" {
                        // ES2015 Number.isFinite() - check if value is finite without coercion
                        // More robust than global isFinite() which coerces to number first
                        if args.len() == 1 {
                            if args[0].is_number() {
                                val = Value::TRUE;
                            } else if let Some(f) = ctx.float_value(args[0]) {
                                val = Value::new_bool(f.is_finite());
                            } else {
                                val = Value::FALSE;
                            }
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Number_isSafeInteger__" {
                        if args.len() == 1 {
                            let max_safe = 9007199254740991.0_f64;
                            let is_safe = if let Some(n) = args[0].int32() {
                                (n as f64).abs() <= max_safe
                            } else if let Some(f) = ctx.float_value(args[0]) {
                                f.is_finite() && f.fract() == 0.0 && f.abs() <= max_safe
                            } else {
                                false
                            };
                            val = Value::new_bool(is_safe);
                        } else {
                            val = Value::FALSE;
                        }
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_number_toFixed__" {
                        let digits = if args.is_empty() {
                            0
                        } else {
                            js_to_int32(ctx, args[0]).unwrap_or(0)
                        };
                        if digits < 0 || digits > 100 {
                            js_throw_error(ctx, JSObjectClassEnum::RangeError, "toFixed() digits out of range");
                            return None;
                        }
                        let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                        let s = format_fixed(n, digits);
                        val = js_new_string(ctx, &s);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_number_toPrecision__" {
                        if args.is_empty() {
                            val = js_to_string(ctx, this_val);
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                        let precision = js_to_int32(ctx, args[0]).unwrap_or(0);
                        if precision < 1 || precision > 100 {
                            js_throw_error(ctx, JSObjectClassEnum::RangeError, "toPrecision() precision out of range");
                            return None;
                        }
                        let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                        let s = format_precision(n, precision);
                        val = js_new_string(ctx, &s);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_number_toExponential__" {
                        let digits_opt = if args.is_empty() {
                            None
                        } else {
                            let d = js_to_int32(ctx, args[0]).unwrap_or(0);
                            if d < 0 || d > 100 {
                                js_throw_error(ctx, JSObjectClassEnum::RangeError, "toExponential() digits out of range");
                                return None;
                            }
                            Some(d)
                        };
                        let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                        let s = format_exponential(n, digits_opt);
                        val = js_new_string(ctx, &s);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_number_toString__" {
                        if args.is_empty() || args[0] == Value::UNDEFINED {
                            val = js_to_string(ctx, this_val);
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                        let radix = js_to_int32(ctx, args[0]).unwrap_or(10);
                        if radix < 2 || radix > 36 {
                            js_throw_error(ctx, JSObjectClassEnum::RangeError, "toString() radix must be between 2 and 36");
                            return None;
                        }
                        let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                        if !n.is_finite() || radix == 10 {
                            val = js_to_string(ctx, this_val);
                        } else {
                            let rounded = n.trunc();
                            if rounded.abs() > (i64::MAX as f64) {
                                val = js_to_string(ctx, this_val);
                            } else {
                                let s = format_radix_int(rounded as i64, radix as u32);
                                val = js_new_string(ctx, &s);
                            }
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
                    } else if marker == "__builtin_String_fromCodePoint__" {
                        if args.is_empty() {
                            val = js_new_string(ctx, "");
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                        let mut result = String::new();
                        for arg in args.iter() {
                            let n = js_to_number(ctx, *arg).unwrap_or(f64::NAN);
                            if !n.is_finite() {
                                js_throw_error(ctx, JSObjectClassEnum::RangeError, "Invalid code point");
                                return None;
                            }
                            let code = n.trunc() as i64;
                            if code < 0 || code > 0x10FFFF {
                                js_throw_error(ctx, JSObjectClassEnum::RangeError, "Invalid code point");
                                return None;
                            }
                            let code_u32 = code as u32;
                            if (0xD800..=0xDFFF).contains(&code_u32) {
                                js_throw_error(ctx, JSObjectClassEnum::RangeError, "Invalid code point");
                                return None;
                            }
                            if let Some(ch) = char::from_u32(code_u32) {
                                result.push(ch);
                            }
                        }
                        val = js_new_string(ctx, &result);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Function_call__" {
                        // Function.prototype.call(thisArg, arg1, arg2, ...)
                        // this_val contains the original function
                        if js_is_function(ctx, this_val) != 0 {
                            let new_this = if args.is_empty() { Value::UNDEFINED } else { args[0] };
                            let call_args: Vec<JSValue> = args.iter().skip(1).copied().collect();
                            if let Some(result) = crate::parser::call_closure_with_this(ctx, this_val, new_this, &call_args) {
                                val = result;
                            } else {
                                val = Value::UNDEFINED;
                            }
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                    } else if marker == "__builtin_Function_apply__" {
                        // Function.prototype.apply(thisArg, argsArray)
                        // this_val contains the original function
                        if js_is_function(ctx, this_val) != 0 {
                            let new_this = if args.is_empty() { Value::UNDEFINED } else { args[0] };
                            let call_args: Vec<JSValue> = if args.len() >= 2 && ctx.object_class_id(args[1]).is_some() {
                                // args[1] is an array-like object
                                let arr = args[1];
                                let len = js_get_property_str(ctx, arr, "length");
                                let len_val = len.int32().unwrap_or(0) as usize;
                                (0..len_val).map(|i| js_get_property_uint32(ctx, arr, i as u32)).collect()
                            } else {
                                Vec::new()
                            };
                            if let Some(result) = crate::parser::call_closure_with_this(ctx, this_val, new_this, &call_args) {
                                val = result;
                            } else {
                                val = Value::UNDEFINED;
                            }
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                    } else if marker == "__builtin_Function_bind__" {
                        // Function.prototype.bind(thisArg, arg1, ...)
                        // Returns a new function with bound this and arguments
                        // this_val contains the original function
                        if js_is_function(ctx, this_val) != 0 {
                            // Create a bound function object
                            let bound_this = if args.is_empty() { Value::UNDEFINED } else { args[0] };
                            let bound_args: Vec<JSValue> = args.iter().skip(1).copied().collect();

                            // Create a wrapper function that calls the original with bound this and args
                            // For now, we create an object that stores the bound values
                            let bound_func = js_new_object(ctx);
                            js_set_property_str(ctx, bound_func, "__bound_func__", this_val);
                            js_set_property_str(ctx, bound_func, "__bound_this__", bound_this);

                            // Store bound args as an array
                            let bound_args_arr = js_new_array(ctx, bound_args.len() as i32);
                            for (i, arg) in bound_args.iter().enumerate() {
                                js_set_property_uint32(ctx, bound_args_arr, i as u32, *arg);
                            }
                            js_set_property_str(ctx, bound_func, "__bound_args__", bound_args_arr);

                            // Mark it as a bound function
                            js_set_property_str(ctx, bound_func, "__is_bound__", Value::TRUE);

                            val = bound_func;
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                    }
                }
            }

            // Check if val is a bound function (created by bind())
            if let Some(class_id) = ctx.object_class_id(val) {
                if class_id == JSObjectClassEnum::Object as u32 {
                    let is_bound = js_get_property_str(ctx, val, "__is_bound__");
                    if is_bound == Value::TRUE {
                        let orig_func = js_get_property_str(ctx, val, "__bound_func__");
                        let bound_this = js_get_property_str(ctx, val, "__bound_this__");
                        let bound_args_arr = js_get_property_str(ctx, val, "__bound_args__");

                        // Collect bound args
                        let bound_args_len = js_get_property_str(ctx, bound_args_arr, "length")
                            .int32().unwrap_or(0) as usize;
                        let mut all_args: Vec<JSValue> = (0..bound_args_len)
                            .map(|i| js_get_property_uint32(ctx, bound_args_arr, i as u32))
                            .collect();
                        // Append call args
                        all_args.extend(args.iter().copied());

                        if let Some(result) = crate::parser::call_closure_with_this(ctx, orig_func, bound_this, &all_args) {
                            val = result;
                        } else {
                            val = Value::UNDEFINED;
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

            if let Some(bytes) = ctx.string_bytes(val) {
                if let Ok(marker) = core::str::from_utf8(bytes) {
                    if marker == "__builtin_Date__" {
                        if name == "now" {
                            val = js_new_string(ctx, "__builtin_Date_now__");
                            rest = next;
                            continue;
                        }
                    } else if marker == "__builtin_console__" {
                        if name == "log" {
                            val = js_new_string(ctx, "__builtin_console_log__");
                            rest = next;
                            continue;
                        }
                    }
                }
            }
            
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
                if ctx.string_bytes(val).is_some() {
                    let len = string_utf16_len(ctx, val).unwrap_or(0) as i32;
                    val = Value::from_int32(len);
                    rest = next;
                    continue;
                }
            }
            
            // Array.push - create a callable wrapper
            if name == "push" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_push__");
                        rest = next;
                        continue;
                    }
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
            
            // String.concat
            if name == "concat" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_concat__");
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

            // String.substr
            if name == "substr" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_substr__");
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

            // String.lastIndexOf
            if name == "lastIndexOf" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_lastIndexOf__");
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

            // TypedArray methods
            if let Some(class_id) = ctx.object_class_id(val) {
                if typed_array_kind_from_class_id(class_id).is_some() {
                    if name == "set" {
                        val = js_new_string(ctx, "__builtin_typedarray_set__");
                        rest = next;
                        continue;
                    }
                    if name == "subarray" {
                        val = js_new_string(ctx, "__builtin_typedarray_subarray__");
                        rest = next;
                        continue;
                    }
                    if name == "toString" {
                        val = js_new_string(ctx, "__builtin_typedarray_toString__");
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

            // Number formatting methods
            if name == "toFixed" || name == "toPrecision" || name == "toExponential" || name == "toString" {
                if js_is_number(ctx, val) != 0 {
                    if name == "toFixed" {
                        val = js_new_string(ctx, "__builtin_number_toFixed__");
                    } else if name == "toPrecision" {
                        val = js_new_string(ctx, "__builtin_number_toPrecision__");
                    } else if name == "toString" {
                        val = js_new_string(ctx, "__builtin_number_toString__");
                    } else {
                        val = js_new_string(ctx, "__builtin_number_toExponential__");
                    }
                    rest = next;
                    continue;
                }
            }

            // Array.splice
            if name == "splice" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_splice__");
                        rest = next;
                        continue;
                    }
                }
            }

            // Array iteration methods
            if name == "forEach" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_forEach__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "map" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_map__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "filter" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_filter__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "reduce" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_reduce__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "reduceRight" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_reduceRight__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "every" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_every__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "some" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_some__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "find" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_find__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "findIndex" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_findIndex__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "flat" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_flat__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "flatMap" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_flatMap__");
                        rest = next;
                        continue;
                    }
                }
            }
            if name == "sort" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_sort__");
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

            // String.toLocaleUpperCase (stub)
            if name == "toLocaleUpperCase" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_toLocaleUpperCase__");
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

            // String.toLocaleLowerCase (stub)
            if name == "toLocaleLowerCase" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_toLocaleLowerCase__");
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

            // String.match
            if name == "match" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_match__");
                    rest = next;
                    continue;
                }
            }

            // String.matchAll
            if name == "matchAll" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_matchAll__");
                    rest = next;
                    continue;
                }
            }

            // String.search
            if name == "search" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_search__");
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
            
            // String.replaceAll
            if name == "replaceAll" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_replaceAll__");
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

            // String.codePointAt
            if name == "codePointAt" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_codePointAt__");
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

            // String.normalize (stub)
            if name == "normalize" {
                if js_is_string(ctx, val) != 0 {
                    val = js_new_string(ctx, "__builtin_string_normalize__");
                    rest = next;
                    continue;
                }
            }

            // Object.hasOwnProperty (instance)
            if name == "hasOwnProperty" {
                if ctx.object_class_id(val).is_some() {
                    if !ctx.has_property_str(val, b"hasOwnProperty") {
                        val = js_new_string(ctx, "__builtin_Object_hasOwnProperty__");
                        rest = next;
                        continue;
                    }
                }
            }

            // RegExp methods
            if let Some(class_id) = ctx.object_class_id(val) {
                if class_id == JSObjectClassEnum::Regexp as u32 {
                    if name == "test" {
                        val = js_new_string(ctx, "__builtin_regexp_test__");
                        rest = next;
                        continue;
                    }
                    if name == "exec" {
                        val = js_new_string(ctx, "__builtin_regexp_exec__");
                        rest = next;
                        continue;
                    }
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
            
            // Array.at
            if name == "at" {
                if let Some(class_id) = ctx.object_class_id(val) {
                    if class_id == JSObjectClassEnum::Array as u32 {
                        val = js_new_string(ctx, "__builtin_array_at__");
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
                            "sin" => {
                                val = js_new_string(ctx, "__builtin_Math_sin__");
                                rest = next;
                                continue;
                            }
                            "cos" => {
                                val = js_new_string(ctx, "__builtin_Math_cos__");
                                rest = next;
                                continue;
                            }
                            "tan" => {
                                val = js_new_string(ctx, "__builtin_Math_tan__");
                                rest = next;
                                continue;
                            }
                            "asin" => {
                                val = js_new_string(ctx, "__builtin_Math_asin__");
                                rest = next;
                                continue;
                            }
                            "acos" => {
                                val = js_new_string(ctx, "__builtin_Math_acos__");
                                rest = next;
                                continue;
                            }
                            "atan" => {
                                val = js_new_string(ctx, "__builtin_Math_atan__");
                                rest = next;
                                continue;
                            }
                            "atan2" => {
                                val = js_new_string(ctx, "__builtin_Math_atan2__");
                                rest = next;
                                continue;
                            }
                            "exp" => {
                                val = js_new_string(ctx, "__builtin_Math_exp__");
                                rest = next;
                                continue;
                            }
                            "log" => {
                                val = js_new_string(ctx, "__builtin_Math_log__");
                                rest = next;
                                continue;
                            }
                            "log2" => {
                                val = js_new_string(ctx, "__builtin_Math_log2__");
                                rest = next;
                                continue;
                            }
                            "log10" => {
                                val = js_new_string(ctx, "__builtin_Math_log10__");
                                rest = next;
                                continue;
                            }
                            "fround" => {
                                val = js_new_string(ctx, "__builtin_Math_fround__");
                                rest = next;
                                continue;
                            }
                            "imul" => {
                                val = js_new_string(ctx, "__builtin_Math_imul__");
                                rest = next;
                                continue;
                            }
                            "clz32" => {
                                val = js_new_string(ctx, "__builtin_Math_clz32__");
                                rest = next;
                                continue;
                            }
                            "E" => {
                                val = number_to_value(ctx, core::f64::consts::E);
                                rest = next;
                                continue;
                            }
                            "PI" => {
                                val = number_to_value(ctx, core::f64::consts::PI);
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
                            "values" => {
                                val = js_new_string(ctx, "__builtin_Object_values__");
                                rest = next;
                                continue;
                            }
                            "entries" => {
                                val = js_new_string(ctx, "__builtin_Object_entries__");
                                rest = next;
                                continue;
                            }
                            "assign" => {
                                val = js_new_string(ctx, "__builtin_Object_assign__");
                                rest = next;
                                continue;
                            }
                            "hasOwnProperty" => {
                                val = js_new_string(ctx, "__builtin_Object_hasOwnProperty__");
                                rest = next;
                                continue;
                            }
                            "create" => {
                                val = js_new_string(ctx, "__builtin_Object_create__");
                                rest = next;
                                continue;
                            }
                            "freeze" => {
                                val = js_new_string(ctx, "__builtin_Object_freeze__");
                                rest = next;
                                continue;
                            }
                            "isSealed" => {
                                val = js_new_string(ctx, "__builtin_Object_isSealed__");
                                rest = next;
                                continue;
                            }
                            "isFrozen" => {
                                val = js_new_string(ctx, "__builtin_Object_isFrozen__");
                                rest = next;
                                continue;
                            }
                            "seal" => {
                                val = js_new_string(ctx, "__builtin_Object_seal__");
                                rest = next;
                                continue;
                            }
                            "defineProperty" => {
                                val = js_new_string(ctx, "__builtin_Object_defineProperty__");
                                rest = next;
                                continue;
                            }
                            "getOwnPropertyDescriptor" => {
                                val = js_new_string(ctx, "__builtin_Object_getOwnPropertyDescriptor__");
                                rest = next;
                                continue;
                            }
                            "getPrototypeOf" => {
                                val = js_new_string(ctx, "__builtin_Object_getPrototypeOf__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    } else if marker == "__builtin_JSON__" {
                        match name {
                            "stringify" => {
                                val = js_new_string(ctx, "__builtin_JSON_stringify__");
                                rest = next;
                                continue;
                            }
                            "parse" => {
                                val = js_new_string(ctx, "__builtin_JSON_parse__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    } else if marker == "__builtin_RegExp__" {
                        match name {
                            "test" => {
                                val = js_new_string(ctx, "__builtin_regexp_test__");
                                rest = next;
                                continue;
                            }
                            "exec" => {
                                val = js_new_string(ctx, "__builtin_regexp_exec__");
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
                            "from" => {
                                val = js_new_string(ctx, "__builtin_Array_from__");
                                rest = next;
                                continue;
                            }
                            "of" => {
                                val = js_new_string(ctx, "__builtin_Array_of__");
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
                            "isNaN" => {
                                val = js_new_string(ctx, "__builtin_Number_isNaN__");
                                rest = next;
                                continue;
                            }
                            "isFinite" => {
                                val = js_new_string(ctx, "__builtin_Number_isFinite__");
                                rest = next;
                                continue;
                            }
                            "isSafeInteger" => {
                                val = js_new_string(ctx, "__builtin_Number_isSafeInteger__");
                                rest = next;
                                continue;
                            }
                            "parseInt" => {
                                val = js_new_string(ctx, "__builtin_parseInt__");
                                rest = next;
                                continue;
                            }
                            "parseFloat" => {
                                val = js_new_string(ctx, "__builtin_parseFloat__");
                                rest = next;
                                continue;
                            }
                            "MAX_VALUE" => {
                                val = number_to_value(ctx, f64::MAX);
                                rest = next;
                                continue;
                            }
                            "MIN_VALUE" => {
                                val = number_to_value(ctx, f64::MIN_POSITIVE);
                                rest = next;
                                continue;
                            }
                            "EPSILON" => {
                                val = number_to_value(ctx, f64::EPSILON);
                                rest = next;
                                continue;
                            }
                            "POSITIVE_INFINITY" => {
                                val = number_to_value(ctx, f64::INFINITY);
                                rest = next;
                                continue;
                            }
                            "NEGATIVE_INFINITY" => {
                                val = number_to_value(ctx, f64::NEG_INFINITY);
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
                            "fromCodePoint" => {
                                val = js_new_string(ctx, "__builtin_String_fromCodePoint__");
                                rest = next;
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
            }

            // Function.prototype.call, apply, bind
            if js_is_function(ctx, val) != 0 {
                if name == "call" {
                    val = js_new_string(ctx, "__builtin_Function_call__");
                    rest = next;
                    continue;
                }
                if name == "apply" {
                    val = js_new_string(ctx, "__builtin_Function_apply__");
                    rest = next;
                    continue;
                }
                if name == "bind" {
                    val = js_new_string(ctx, "__builtin_Function_bind__");
                    rest = next;
                    continue;
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

// ============================================================================
// CLOSURE HANDLING
// ============================================================================
// NOTE: Also available in parser.rs:
// - call_closure() - Extracted and public in parser.rs
// - create_function() - Extracted and public in parser.rs

/// Execute a function body and handle return statements
pub fn eval_function_body(ctx: &mut JSContextImpl, body: &str) -> Option<JSValue> {
    let cleaned = normalize_line_continuations(&strip_comments(body));
    let stmts = split_statements(&cleaned)?;
    let mut last = Value::UNDEFINED;
    
    for stmt in stmts {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        
        // Check for break/continue (with optional label)
        if trimmed == "break" {
            ctx.set_loop_control(crate::context::LoopControl::Break);
            return Some(Value::UNDEFINED);
        }
        if trimmed.starts_with("break ") {
            let label = trimmed[6..].trim();
            if is_identifier(label) {
                ctx.set_loop_control(crate::context::LoopControl::BreakLabel(label.to_string()));
                return Some(Value::UNDEFINED);
            }
        }
        if trimmed == "continue" {
            ctx.set_loop_control(crate::context::LoopControl::Continue);
            return Some(Value::UNDEFINED);
        }
        if trimmed.starts_with("continue ") {
            let label = trimmed[9..].trim();
            if is_identifier(label) {
                ctx.set_loop_control(crate::context::LoopControl::ContinueLabel(label.to_string()));
                return Some(Value::UNDEFINED);
            }
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

        // Check for throw statement
        if trimmed.starts_with("throw ") {
            let expr = &trimmed[6..]; // skip "throw "
            if let Some(val) = eval_expr(ctx, expr.trim()) {
                ctx.set_exception(val);
                return Some(Value::EXCEPTION);
            }
            return None;
        }
        if trimmed == "throw" {
            ctx.set_exception(Value::UNDEFINED);
            return Some(Value::EXCEPTION);
        }

        // Check for try/catch/finally
        if trimmed.starts_with("try ") || trimmed.starts_with("try{") {
            if let Some(val) = parse_try_catch(ctx, trimmed) {
                last = val;
                if *ctx.get_loop_control() != crate::context::LoopControl::None {
                    return Some(last);
                }
                continue;
            }
            return None;
        }

        // Check for function declaration
        if trimmed.starts_with("function ") {
            if let Some(val) = parse_function_declaration(ctx, trimmed) {
                last = val;
                continue;
            }
            return None;
        }

        // Check for labeled statement (label: statement) BEFORE loop/switch checks
        // A labeled statement like "L1: for(...)" starts with the label, not "for"
        if let Some(label_result) = parse_labeled_statement(ctx, trimmed) {
            last = label_result?;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for if statement
        if trimmed.starts_with("if ") || trimmed.starts_with("if(") {
            last = parse_if_statement(ctx, trimmed)?;
            // Check if break/continue was set during statement execution
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for while loop
        if trimmed.starts_with("while ") || trimmed.starts_with("while(") {
            last = parse_while_loop(ctx, trimmed, None)?;
            // Check if break/continue was set during statement execution
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for for loop
        if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
            last = parse_for_loop(ctx, trimmed, None)?;
            // Check if break/continue was set during statement execution
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for do...while loop
        if trimmed.starts_with("do ") || trimmed.starts_with("do{") {
            last = parse_do_while_loop(ctx, trimmed, None)?;
            // Check if break/continue was set during statement execution
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for switch statement
        if trimmed.starts_with("switch ") || trimmed.starts_with("switch(") {
            last = parse_switch_statement(ctx, trimmed)?;
            // Check if break/continue was set during statement execution
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Check for bare block statement: { ... }
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            // Verify this looks like a block, not an object literal
            // Object literals like { x: 1 } in expression position would have a colon
            // Blocks have statements, not key-value pairs
            let (block_content, _) = extract_braces(trimmed)?;
            last = eval_block(ctx, block_content)?;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        
        // Execute statement
        last = eval_expr(ctx, trimmed)?;
        if last.is_exception() {
            return Some(last);
        }
        
        // Check if break/continue was set during statement execution
        if *ctx.get_loop_control() != crate::context::LoopControl::None {
            return Some(last);
        }
    }
    
    Some(last)
}

// ============================================================================
// STATEMENT PARSING
// ============================================================================
// NOTE: All statement parsing has been EXTRACTED to parser.rs (1,270 lines):
// - parse_function_declaration() - Function definitions
// - parse_if_statement() - if/else statements
// - parse_while_loop() - while loops
// - parse_for_loop() - for/for-in/for-of loops
// - parse_do_while_loop() - do-while loops
// - parse_switch_statement() - switch/case statements
// - parse_try_catch() - try/catch/finally statements
// - parse_lvalue() - Left-value parsing for assignments
// - extract_braces(), extract_paren(), extract_bracket() - Delimiter extraction
// - split_assignment(), split_ternary(), split_base_and_tail() - Expression splitting
//
// Parsing helpers live in parser.rs and are imported above.


pub fn eval_program(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let cleaned = normalize_line_continuations(&strip_comments(src));
    let stmts = split_statements(&cleaned)?;
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
            if let Some(val) = parse_while_loop(ctx, trimmed, None) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for for loop
        if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
            if let Some(val) = parse_for_loop(ctx, trimmed, None) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for do...while loop
        if trimmed.starts_with("do ") || trimmed.starts_with("do{") {
            if let Some(val) = parse_do_while_loop(ctx, trimmed, None) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for switch statement
        if trimmed.starts_with("switch ") || trimmed.starts_with("switch(") {
            if let Some(val) = parse_switch_statement(ctx, trimmed) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for throw statement
        if trimmed.starts_with("throw ") {
            let expr = &trimmed[6..]; // skip "throw "
            if let Some(val) = eval_expr(ctx, expr.trim()) {
                ctx.set_exception(val);
                return Some(Value::EXCEPTION);
            }
            return None;
        }
        // Check for throw statement
        if trimmed.starts_with("throw ") || trimmed == "throw" {
            if trimmed == "throw" {
                ctx.set_exception(Value::UNDEFINED);
                return Some(Value::EXCEPTION);
            }
            let expr = trimmed[6..].trim();
            if let Some(val) = eval_expr(ctx, expr) {
                ctx.set_exception(val);
                return Some(Value::EXCEPTION);
            }
            ctx.set_exception(Value::UNDEFINED);
            return Some(Value::EXCEPTION);
        }
        // Check for try/catch/finally
        if trimmed.starts_with("try ") || trimmed.starts_with("try{") {
            if let Some(val) = parse_try_catch(ctx, trimmed) {
                last = val;
                any = true;
                continue;
            }
            return None;
        }
        // Check for labeled statement (label: statement) BEFORE bare blocks
        if let Some(label_result) = parse_labeled_statement(ctx, trimmed) {
            last = label_result?;
            any = true;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        // Check for bare block statement: { ... }
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            let (block_content, _) = extract_braces(trimmed)?;
            last = eval_block(ctx, block_content)?;
            any = true;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }
        last = eval_expr(ctx, trimmed)?;
        if last.is_exception() {
            return Some(last);
        }
        any = true;
    }
    if any { Some(last) } else { None }
}

// ============================================================================
// JSON PARSING
// ============================================================================
// NOTE: JSON functionality has been EXTRACTED to json.rs (388 lines):
// - parse_json() - Main JSON parser (also available in json.rs as public API)
// - json_stringify_value() - Value to JSON string conversion
// - JsonParser struct - Full JSON parsing implementation
//
// These helper functions remain here for internal use by eval_expr.





// ============================================================================
// DELIMITER EXTRACTION HELPERS
// ============================================================================
// NOTE: These functions are also EXTRACTED to parser.rs:
// - extract_paren() - Extract content within ()
// - extract_bracket() - Extract content within []
// - extract_braces() - Extract content within {}



// ============================================================================
// C FUNCTION CALL INFRASTRUCTURE
// ============================================================================
// Handles calls to C functions registered via JS_SetCFunctionTable.
// Supports multiple calling conventions (generic, constructor, magic, etc.)

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

// ============================================================================
// LITERAL EVALUATION HELPERS
// ============================================================================
// NOTE: These functions are also EXTRACTED to evals.rs:
// - eval_array_literal() - Parse [1,2,3] syntax
// - eval_object_literal() - Parse {a:1, b:2} syntax
// - split_top_level() - Split comma-separated lists




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
        if self.peek() == Some(b'0') {
            if let Some(next) = self.input.get(self.pos + 1).copied() {
                let (radix, is_prefix) = match next {
                    b'x' | b'X' => (16, true),
                    b'o' | b'O' => (8, true),
                    b'b' | b'B' => (2, true),
                    _ => (10, false),
                };
                if is_prefix {
                    self.pos += 2;
                    let start_digits = self.pos;
                    while let Some(b) = self.peek() {
                        let ok = match radix {
                            16 => b.is_ascii_hexdigit(),
                            8 => matches!(b, b'0'..=b'7'),
                            2 => matches!(b, b'0' | b'1'),
                            _ => b.is_ascii_digit(),
                        };
                        if ok {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    if self.pos == start_digits {
                        return Err(());
                    }
                    let slice = &self.input[start_digits..self.pos];
                    let s = core::str::from_utf8(slice).map_err(|_| ())?;
                    let v = u64::from_str_radix(s, radix).map_err(|_| ())?;
                    return Ok(v as f64);
                }
            }
        }
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
        if self.starts_with_keyword(b"typeof") {
            self.pos += 6;
            self.skip_ws();
            let ctx = unsafe { &mut *self.ctx };
            if let Ok(rest) = core::str::from_utf8(&self.input[self.pos..]) {
                if let Some((name, remaining)) = parse_identifier(rest) {
                    if ctx.resolve_binding(name).is_none() {
                        let global = js_get_global_object(ctx);
                        let gv = js_get_property_str(ctx, global, name);
                        if gv.is_undefined() && !ctx.has_property_str(global, name.as_bytes()) {
                            self.pos += rest.len() - remaining.len();
                            return Ok(js_new_string(ctx, "undefined"));
                        }
                    }
                }
            }
            let val = self.parse_unary()?;
            return self.typeof_value(val);
        }
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
        let mut this_val = Value::UNDEFINED;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'.') => {
                    self.pos += 1;
                    let rest = core::str::from_utf8(&self.input[self.pos..]).map_err(|_| ())?;
                    let (prop, remaining) = parse_identifier(rest).ok_or(())?;
                    let consumed = rest.len() - remaining.len();
                    self.pos += consumed;
                    this_val = value;
                    let ctx = unsafe { &mut *self.ctx };

                    // Check for built-in string methods
                    if js_is_string(ctx, value) != 0 {
                        if let Some(bytes) = ctx.string_bytes(value) {
                            if let Ok(marker) = core::str::from_utf8(bytes) {
                                if marker == "__builtin_Number__" {
                                    value = match prop {
                                        "isInteger" => js_new_string(ctx, "__builtin_Number_isInteger__"),
                                        "isNaN" => js_new_string(ctx, "__builtin_Number_isNaN__"),
                                        "isFinite" => js_new_string(ctx, "__builtin_Number_isFinite__"),
                                        "isSafeInteger" => js_new_string(ctx, "__builtin_Number_isSafeInteger__"),
                                        "parseInt" => js_new_string(ctx, "__builtin_parseInt__"),
                                        "parseFloat" => js_new_string(ctx, "__builtin_parseFloat__"),
                                        "MAX_VALUE" => number_to_value(ctx, f64::MAX),
                                        "MIN_VALUE" => number_to_value(ctx, f64::MIN_POSITIVE),
                                        "EPSILON" => number_to_value(ctx, f64::EPSILON),
                                        "POSITIVE_INFINITY" => number_to_value(ctx, f64::INFINITY),
                                        "NEGATIVE_INFINITY" => number_to_value(ctx, f64::NEG_INFINITY),
                                        _ => value,
                                    };
                                    continue;
                                }
                                if marker == "__builtin_Math__" {
                                    value = match prop {
                                        "floor" => js_new_string(ctx, "__builtin_Math_floor__"),
                                        "ceil" => js_new_string(ctx, "__builtin_Math_ceil__"),
                                        "round" => js_new_string(ctx, "__builtin_Math_round__"),
                                        "abs" => js_new_string(ctx, "__builtin_Math_abs__"),
                                        "max" => js_new_string(ctx, "__builtin_Math_max__"),
                                        "min" => js_new_string(ctx, "__builtin_Math_min__"),
                                        "sqrt" => js_new_string(ctx, "__builtin_Math_sqrt__"),
                                        "pow" => js_new_string(ctx, "__builtin_Math_pow__"),
                                        "sin" => js_new_string(ctx, "__builtin_Math_sin__"),
                                        "cos" => js_new_string(ctx, "__builtin_Math_cos__"),
                                        "tan" => js_new_string(ctx, "__builtin_Math_tan__"),
                                        "asin" => js_new_string(ctx, "__builtin_Math_asin__"),
                                        "acos" => js_new_string(ctx, "__builtin_Math_acos__"),
                                        "atan" => js_new_string(ctx, "__builtin_Math_atan__"),
                                        "atan2" => js_new_string(ctx, "__builtin_Math_atan2__"),
                                        "exp" => js_new_string(ctx, "__builtin_Math_exp__"),
                                        "log" => js_new_string(ctx, "__builtin_Math_log__"),
                                        "log2" => js_new_string(ctx, "__builtin_Math_log2__"),
                                        "log10" => js_new_string(ctx, "__builtin_Math_log10__"),
                                        "fround" => js_new_string(ctx, "__builtin_Math_fround__"),
                                        "imul" => js_new_string(ctx, "__builtin_Math_imul__"),
                                        "clz32" => js_new_string(ctx, "__builtin_Math_clz32__"),
                                        "E" => number_to_value(ctx, core::f64::consts::E),
                                        "PI" => number_to_value(ctx, core::f64::consts::PI),
                                        _ => value,
                                    };
                                    continue;
                                }
                                if marker == "__builtin_String__" {
                                    value = match prop {
                                        "fromCharCode" => js_new_string(ctx, "__builtin_String_fromCharCode__"),
                                        "fromCodePoint" => js_new_string(ctx, "__builtin_String_fromCodePoint__"),
                                        _ => value,
                                    };
                                    continue;
                                }
                                if marker == "__builtin_JSON__" {
                                    value = match prop {
                                        "stringify" => js_new_string(ctx, "__builtin_JSON_stringify__"),
                                        "parse" => js_new_string(ctx, "__builtin_JSON_parse__"),
                                        _ => value,
                                    };
                                    continue;
                                }
                                if marker == "__builtin_Date__" {
                                    if prop == "now" {
                                        value = js_new_string(ctx, "__builtin_Date_now__");
                                        continue;
                                    }
                                }
                                if marker == "__builtin_console__" {
                                    if prop == "log" {
                                        value = js_new_string(ctx, "__builtin_console_log__");
                                        continue;
                                    }
                                }
                            }
                        }
                        value = match prop {
                            "charAt" => js_new_string(ctx, "__builtin_string_charAt__"),
                            "toUpperCase" => js_new_string(ctx, "__builtin_string_toUpperCase__"),
                            "toLowerCase" => js_new_string(ctx, "__builtin_string_toLowerCase__"),
                            "substring" => js_new_string(ctx, "__builtin_string_substring__"),
                            "substr" => js_new_string(ctx, "__builtin_string_substr__"),
                            "slice" => js_new_string(ctx, "__builtin_string_slice__"),
                            "indexOf" => js_new_string(ctx, "__builtin_string_indexOf__"),
                            "lastIndexOf" => js_new_string(ctx, "__builtin_string_lastIndexOf__"),
                            "split" => js_new_string(ctx, "__builtin_string_split__"),
                            "concat" => js_new_string(ctx, "__builtin_string_concat__"),
                            "trim" => js_new_string(ctx, "__builtin_string_trim__"),
                            "trimStart" => js_new_string(ctx, "__builtin_string_trimStart__"),
                            "trimEnd" => js_new_string(ctx, "__builtin_string_trimEnd__"),
                            "includes" => js_new_string(ctx, "__builtin_string_includes__"),
                            "startsWith" => js_new_string(ctx, "__builtin_string_startsWith__"),
                            "endsWith" => js_new_string(ctx, "__builtin_string_endsWith__"),
                            "repeat" => js_new_string(ctx, "__builtin_string_repeat__"),
                            "replace" => js_new_string(ctx, "__builtin_string_replace__"),
                            "replaceAll" => js_new_string(ctx, "__builtin_string_replaceAll__"),
                            "match" => js_new_string(ctx, "__builtin_string_match__"),
                            "matchAll" => js_new_string(ctx, "__builtin_string_matchAll__"),
                            "search" => js_new_string(ctx, "__builtin_string_search__"),
                            "charCodeAt" => js_new_string(ctx, "__builtin_string_charCodeAt__"),
                            "codePointAt" => js_new_string(ctx, "__builtin_string_codePointAt__"),
                            "padStart" => js_new_string(ctx, "__builtin_string_padStart__"),
                            "padEnd" => js_new_string(ctx, "__builtin_string_padEnd__"),
                            "length" => {
                                let len = string_utf16_len(ctx, value).unwrap_or(0);
                                Value::from_int32(len as i32)
                            }
                            _ => js_get_property_str(ctx, value, prop),
                        };
                    } else if js_is_number(ctx, value) != 0 {
                        value = match prop {
                            "toFixed" => js_new_string(ctx, "__builtin_number_toFixed__"),
                            "toPrecision" => js_new_string(ctx, "__builtin_number_toPrecision__"),
                            "toExponential" => js_new_string(ctx, "__builtin_number_toExponential__"),
                            "toString" => js_new_string(ctx, "__builtin_number_toString__"),
                            _ => js_get_property_str(ctx, value, prop),
                        };
                    } else if let Some(class_id) = ctx.object_class_id(value) {
                        // Check for built-in array methods
                        if class_id == JSObjectClassEnum::Array as u32 {
                            value = match prop {
                                "push" => js_new_string(ctx, "__builtin_array_push__"),
                                "pop" => js_new_string(ctx, "__builtin_array_pop__"),
                                "join" => js_new_string(ctx, "__builtin_array_join__"),
                                "slice" => js_new_string(ctx, "__builtin_array_slice__"),
                                "indexOf" => js_new_string(ctx, "__builtin_array_indexOf__"),
                                "splice" => js_new_string(ctx, "__builtin_array_splice__"),
                                "forEach" => js_new_string(ctx, "__builtin_array_forEach__"),
                                "map" => js_new_string(ctx, "__builtin_array_map__"),
                                "filter" => js_new_string(ctx, "__builtin_array_filter__"),
                                "reduce" => js_new_string(ctx, "__builtin_array_reduce__"),
                                "reduceRight" => js_new_string(ctx, "__builtin_array_reduceRight__"),
                                "every" => js_new_string(ctx, "__builtin_array_every__"),
                                "some" => js_new_string(ctx, "__builtin_array_some__"),
                                "find" => js_new_string(ctx, "__builtin_array_find__"),
                                "findIndex" => js_new_string(ctx, "__builtin_array_findIndex__"),
                                "flat" => js_new_string(ctx, "__builtin_array_flat__"),
                                "flatMap" => js_new_string(ctx, "__builtin_array_flatMap__"),
                                "sort" => js_new_string(ctx, "__builtin_array_sort__"),
                                "length" => js_get_property_str(ctx, value, "length"),
                                _ => js_get_property_str(ctx, value, prop),
                            };
                        } else if typed_array_kind_from_class_id(class_id).is_some() {
                            value = match prop {
                                "set" => js_new_string(ctx, "__builtin_typedarray_set__"),
                                "subarray" => js_new_string(ctx, "__builtin_typedarray_subarray__"),
                                "toString" => js_new_string(ctx, "__builtin_typedarray_toString__"),
                                _ => js_get_property_str(ctx, value, prop),
                            };
                        } else {
                            value = js_get_property_str(ctx, value, prop);
                        }
                    } else {
                        value = js_get_property_str(ctx, value, prop);
                    }

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
                    this_val = value;
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
                Some(b'(') => {
                    // Function call - parse arguments
                    self.pos += 1;
                    let mut args = Vec::new();
                    self.skip_ws();
                    if self.peek() != Some(b')') {
                        loop {
                            let arg = self.parse_expr()?;
                            args.push(arg);
                            self.skip_ws();
                            match self.peek() {
                                Some(b',') => {
                                    self.pos += 1;
                                    self.skip_ws();
                                }
                                Some(b')') => break,
                                _ => return Err(()),
                            }
                        }
                    }
                    if self.peek() != Some(b')') {
                        return Err(());
                    }
                    self.pos += 1;

                    // Call the method using the builtin dispatch
                    let ctx = unsafe { &mut *self.ctx };
                    value = self.call_builtin_method(ctx, value, this_val, &args)?;
                    this_val = Value::UNDEFINED;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn call_builtin_method(&mut self, ctx: &mut JSContextImpl, method: JSValue, this_val: JSValue, args: &[JSValue]) -> Result<JSValue, ()> {
        // Check if method is a builtin marker string
        if let Some(bytes) = ctx.string_bytes(method) {
            if let Ok(marker) = core::str::from_utf8(bytes) {
                let marker = marker.to_string();
                if marker == "__builtin_eval__" {
                    if let Some(val) = call_builtin_global_marker(ctx, &marker, args) {
                        return Ok(val);
                    }
                }
                if marker == "__builtin_Date_now__" {
                    return Ok(js_date_now(ctx));
                }
                if marker == "__builtin_console_log__" {
                    js_console_log(ctx, args);
                    return Ok(Value::UNDEFINED);
                }
                // String methods
                if marker == "__builtin_string_charAt__" {
                    if args.len() == 1 {
                        if let Some(idx) = args[0].int32() {
                            if let Some(units) = string_utf16_units(ctx, this_val) {
                                if idx >= 0 && (idx as usize) < units.len() {
                                    let s = String::from_utf16_lossy(&[units[idx as usize]]);
                                    return Ok(js_new_string(ctx, &s));
                                }
                                return Ok(js_new_string(ctx, ""));
                            }
                        }
                    }
                } else if marker == "__builtin_string_toUpperCase__" {
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            let upper = s.to_uppercase();
                            return Ok(js_new_string(ctx, &upper));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_toLowerCase__" {
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            let lower = s.to_lowercase();
                            return Ok(js_new_string(ctx, &lower));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_toLocaleUpperCase__" {
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            let upper = s.to_uppercase();
                            return Ok(js_new_string(ctx, &upper));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_toLocaleLowerCase__" {
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            let lower = s.to_lowercase();
                            return Ok(js_new_string(ctx, &lower));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_normalize__" {
                    return Ok(this_val);
                } else if marker == "__builtin_string_substring__" {
                    if args.len() >= 1 && args.len() <= 2 {
                        if let Some(units) = string_utf16_units(ctx, this_val) {
                            let len = units.len() as i32;
                            let mut start = args[0].int32().unwrap_or(0).max(0).min(len);
                            let mut end = if args.len() == 2 {
                                args[1].int32().unwrap_or(len).max(0).min(len)
                            } else {
                                len
                            };
                            if start > end {
                                core::mem::swap(&mut start, &mut end);
                            }
                            let s = String::from_utf16_lossy(&units[start as usize..end as usize]);
                            return Ok(js_new_string(ctx, &s));
                        }
                    }
                    return Ok(js_new_string(ctx, ""));
                } else if marker == "__builtin_string_substr__" {
                    if args.len() >= 1 {
                        if let Some(units) = string_utf16_units(ctx, this_val) {
                            let len = units.len() as i32;
                            let mut start = args[0].int32().unwrap_or(0);
                            if start < 0 {
                                start = (len + start).max(0);
                            } else if start > len {
                                start = len;
                            }
                            let count = if args.len() >= 2 {
                                args[1].int32().unwrap_or(0).max(0)
                            } else {
                                len - start
                            };
                            let end = (start + count).min(len);
                            let s = String::from_utf16_lossy(&units[start as usize..end as usize]);
                            return Ok(js_new_string(ctx, &s));
                        }
                    }
                    return Ok(js_new_string(ctx, ""));
                } else if marker == "__builtin_string_substr__" {
                    if args.len() >= 1 {
                        if let Some(units) = string_utf16_units(ctx, this_val) {
                            let len = units.len() as i32;
                            let mut start = args[0].int32().unwrap_or(0);
                            if start < 0 {
                                start = (len + start).max(0);
                            } else if start > len {
                                start = len;
                            }
                            let count = if args.len() >= 2 {
                                args[1].int32().unwrap_or(0).max(0)
                            } else {
                                len - start
                            };
                            let end = (start + count).min(len);
                            let s = String::from_utf16_lossy(&units[start as usize..end as usize]);
                            return Ok(js_new_string(ctx, &s));
                        }
                    }
                    return Ok(js_new_string(ctx, ""));
                } else if marker == "__builtin_string_slice__" {
                    if args.len() >= 1 && args.len() <= 2 {
                        if let Some(units) = string_utf16_units(ctx, this_val) {
                            let len = units.len() as i32;
                            let mut start = if args.len() >= 1 { args[0].int32().unwrap_or(0) } else { 0 };
                            let mut end = if args.len() == 2 { args[1].int32().unwrap_or(len) } else { len };
                            if start < 0 {
                                start = (len + start).max(0);
                            } else {
                                start = start.min(len);
                            }
                            if end < 0 {
                                end = (len + end).max(0);
                            } else {
                                end = end.min(len);
                            }
                            if end < start {
                                end = start;
                            }
                            let s = String::from_utf16_lossy(&units[start as usize..end as usize]);
                            return Ok(js_new_string(ctx, &s));
                        }
                    }
                } else if marker == "__builtin_string_indexOf__" {
                    if args.len() >= 1 {
                        if let Some(haystack_units) = string_utf16_units(ctx, this_val) {
                            let needle_str = value_to_string(ctx, args[0]);
                            let needle_units: Vec<u16> = needle_str.encode_utf16().collect();
                            let len = haystack_units.len() as i32;
                            let mut start = if args.len() >= 2 { js_to_int32(ctx, args[1]).unwrap_or(0) } else { 0 };
                            if start < 0 {
                                start = 0;
                            }
                            if needle_units.is_empty() {
                                return Ok(Value::from_int32(start.min(len)));
                            }
                            let mut found = -1;
                            let start_u = start.min(len) as usize;
                            if needle_units.len() <= haystack_units.len().saturating_sub(start_u) {
                                for i in start_u..=haystack_units.len().saturating_sub(needle_units.len()) {
                                    if haystack_units[i..i + needle_units.len()] == needle_units[..] {
                                        found = i as i32;
                                        break;
                                    }
                                }
                            }
                            return Ok(Value::from_int32(found));
                        }
                    }
                } else if marker == "__builtin_string_lastIndexOf__" {
                    if args.len() >= 1 {
                        if let Some(haystack_units) = string_utf16_units(ctx, this_val) {
                            let needle_str = value_to_string(ctx, args[0]);
                            let needle_units: Vec<u16> = needle_str.encode_utf16().collect();
                            let len = haystack_units.len() as i32;
                            let mut start = if args.len() >= 2 {
                                js_to_int32(ctx, args[1]).unwrap_or(len - 1)
                            } else {
                                len - 1
                            };
                            if start >= len {
                                start = len - 1;
                            }
                            if start < 0 {
                                start = len + start;
                            }
                            let mut found = -1;
                            if needle_units.is_empty() {
                                found = start.max(-1);
                            } else if start >= 0 {
                                let mut i = (start as usize).min(haystack_units.len().saturating_sub(needle_units.len()));
                                loop {
                                    if i + needle_units.len() <= haystack_units.len()
                                        && haystack_units[i..i + needle_units.len()] == needle_units[..] {
                                        found = i as i32;
                                        break;
                                    }
                                    if i == 0 { break; }
                                    i -= 1;
                                }
                            }
                            return Ok(Value::from_int32(found));
                        }
                    }
                } else if marker == "__builtin_string_split__" {
                    if args.len() >= 1 {
                        if let Some(str_bytes) = ctx.string_bytes(this_val) {
                            let str_owned = str_bytes.to_vec();
                            if let Some(sep_bytes) = ctx.string_bytes(args[0]) {
                                let sep_owned = sep_bytes.to_vec();
                                if let (Ok(s), Ok(sep)) = (core::str::from_utf8(&str_owned), core::str::from_utf8(&sep_owned)) {
                                    let arr = js_new_array(ctx, 0);
                                    let parts: Vec<&str> = s.split(sep).collect();
                                    for (i, part) in parts.iter().enumerate() {
                                        let part_val = js_new_string(ctx, part);
                                        js_set_property_uint32(ctx, arr, i as u32, part_val);
                                    }
                                    return Ok(arr);
                                }
                            }
                        }
                    }
                } else if marker == "__builtin_string_concat__" {
                    let mut result = if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            s.to_string()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    for arg in args {
                        if let Some(arg_bytes) = ctx.string_bytes(*arg) {
                            if let Ok(s) = core::str::from_utf8(arg_bytes) {
                                result.push_str(s);
                            }
                        } else if let Some(n) = arg.int32() {
                            result.push_str(&n.to_string());
                        }
                    }
                    return Ok(js_new_string(ctx, &result));
                } else if marker == "__builtin_string_trim__" {
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        let owned = str_bytes.to_vec();
                        if let Ok(s) = core::str::from_utf8(&owned) {
                            return Ok(js_new_string(ctx, s.trim()));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_trimStart__" {
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        let owned = str_bytes.to_vec();
                        if let Ok(s) = core::str::from_utf8(&owned) {
                            return Ok(js_new_string(ctx, s.trim_start()));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_trimEnd__" {
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        let owned = str_bytes.to_vec();
                        if let Ok(s) = core::str::from_utf8(&owned) {
                            return Ok(js_new_string(ctx, s.trim_end()));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_startsWith__" {
                    if args.len() == 1 {
                        let s = value_to_string(ctx, this_val);
                        let prefix = value_to_string(ctx, args[0]);
                        return Ok(Value::new_bool(s.starts_with(&prefix)));
                    }
                    return Ok(Value::FALSE);
                } else if marker == "__builtin_string_endsWith__" {
                    if args.len() == 1 {
                        let s = value_to_string(ctx, this_val);
                        let suffix = value_to_string(ctx, args[0]);
                        return Ok(Value::new_bool(s.ends_with(&suffix)));
                    }
                    return Ok(Value::FALSE);
                } else if marker == "__builtin_string_includes__" {
                    if args.len() == 1 {
                        let s = value_to_string(ctx, this_val);
                        let search = value_to_string(ctx, args[0]);
                        return Ok(Value::new_bool(s.contains(&search)));
                    }
                    return Ok(Value::FALSE);
                } else if marker == "__builtin_string_repeat__" {
                    if args.len() == 1 {
                        if let Some(count) = args[0].int32() {
                            let s = value_to_string(ctx, this_val);
                            return Ok(js_new_string(ctx, &s.repeat(count.max(0) as usize)));
                        }
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_match__" {
                    if args.is_empty() {
                        return Ok(Value::NULL);
                    }
                    let input_val = coerce_to_string_value(ctx, this_val);
                    let s = value_to_string(ctx, input_val);
                    let (pattern, flags) = if let Some((src, flg)) = regexp_parts(ctx, args[0]) {
                        (src, flg)
                    } else {
                        (value_to_string(ctx, args[0]), String::new())
                    };
                    let (re, global) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                    if global {
                        let mut matches = Vec::new();
                        for m in re.find_iter(&s) {
                            match m {
                                Ok(mm) => matches.push(mm.as_str().to_string()),
                                Err(_) => return Err(()),
                            }
                        }
                        if matches.is_empty() {
                            return Ok(Value::NULL);
                        }
                        let arr = js_new_array(ctx, matches.len() as i32);
                        for (i, m) in matches.iter().enumerate() {
                            let mv = js_new_string(ctx, m);
                            js_set_property_uint32(ctx, arr, i as u32, mv);
                        }
                        return Ok(arr);
                    }
                    if let Ok(Some(caps)) = re.captures(&s) {
                        let arr = js_new_array(ctx, caps.len() as i32);
                        for i in 0..caps.len() {
                            if let Some(m) = caps.get(i) {
                                let mv = js_new_string(ctx, m.as_str());
                                js_set_property_uint32(ctx, arr, i as u32, mv);
                            } else {
                                js_set_property_uint32(ctx, arr, i as u32, Value::UNDEFINED);
                            }
                        }
                        let idx = caps.get(0).map(|m| m.start() as i32).unwrap_or(0);
                        let _ = js_set_property_str(ctx, arr, "index", Value::from_int32(idx));
                        let _ = js_set_property_str(ctx, arr, "input", input_val);
                        return Ok(arr);
                    }
                    return Ok(Value::NULL);
                } else if marker == "__builtin_string_matchAll__" {
                    let input_val = coerce_to_string_value(ctx, this_val);
                    let s = value_to_string(ctx, input_val);
                    let (pattern, flags) = if args.is_empty() {
                        (String::new(), "g".to_string())
                    } else if let Some((src, flg)) = regexp_parts(ctx, args[0]) {
                        (src, flg)
                    } else {
                        (value_to_string(ctx, args[0]), "g".to_string())
                    };
                    let (re, global) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                    if !global {
                        js_throw_error(ctx, JSObjectClassEnum::TypeError, "matchAll requires a global RegExp");
                        return Err(());
                    }
                    let mut matches = Vec::new();
                    for m in re.find_iter(&s) {
                        match m {
                            Ok(mm) => matches.push(mm.as_str().to_string()),
                            Err(_) => return Err(()),
                        }
                    }
                    let arr = js_new_array(ctx, matches.len() as i32);
                    for (i, m) in matches.iter().enumerate() {
                        let mv = js_new_string(ctx, m);
                        js_set_property_uint32(ctx, arr, i as u32, mv);
                    }
                    return Ok(arr);
                } else if marker == "__builtin_string_search__" {
                    if args.is_empty() {
                        return Ok(Value::from_int32(-1));
                    }
                    let s = value_to_string(ctx, this_val);
                    let (pattern, flags) = if let Some((src, flg)) = regexp_parts(ctx, args[0]) {
                        (src, flg)
                    } else {
                        (value_to_string(ctx, args[0]), String::new())
                    };
                    let (re, _) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                    match re.find(&s) {
                        Ok(Some(m)) => return Ok(Value::from_int32(m.start() as i32)),
                        Ok(None) => return Ok(Value::from_int32(-1)),
                        Err(_) => return Err(()),
                    }
                    return Ok(Value::from_int32(-1));
                } else if marker == "__builtin_string_replace__" {
                    if args.len() >= 2 {
                        let s = value_to_string(ctx, this_val);
                        if let Some((pattern, flags)) = regexp_parts(ctx, args[0]) {
                            let (re, global) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                            let replacement = value_to_string(ctx, args[1]);
                            let replaced = if global {
                                re.replace_all(&s, replacement.as_str())
                            } else {
                                re.replace(&s, replacement.as_str())
                            };
                            return Ok(js_new_string(ctx, &replaced.to_string()));
                        }
                        let search = value_to_string(ctx, args[0]);
                        let replacement = value_to_string(ctx, args[1]);
                        return Ok(js_new_string(ctx, &s.replacen(&search, &replacement, 1)));
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_replaceAll__" {
                    if args.len() >= 2 {
                        let s = value_to_string(ctx, this_val);
                        if let Some((pattern, flags)) = regexp_parts(ctx, args[0]) {
                            let (re, global) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                            if !global {
                                js_throw_error(ctx, JSObjectClassEnum::TypeError, "replaceAll requires a global RegExp");
                                return Err(());
                            }
                            let replacement = value_to_string(ctx, args[1]);
                            let replaced = re.replace_all(&s, replacement.as_str());
                            return Ok(js_new_string(ctx, &replaced.to_string()));
                        }
                        let search = value_to_string(ctx, args[0]);
                        let replacement = value_to_string(ctx, args[1]);
                        return Ok(js_new_string(ctx, &s.replace(&search, &replacement)));
                    }
                    return Ok(this_val);
                } else if marker == "__builtin_string_charCodeAt__" {
                    let idx = if args.len() >= 1 {
                        args[0].int32().unwrap_or(0)
                    } else {
                        0
                    };
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            if idx >= 0 && (idx as usize) < s.len() {
                                if let Some(ch) = s.chars().nth(idx as usize) {
                                    return Ok(number_to_value(ctx, ch as u32 as f64));
                                }
                            }
                        }
                    }
                    return Ok(number_to_value(ctx, f64::NAN));
                } else if marker == "__builtin_string_codePointAt__" {
                    let idx = if args.len() >= 1 {
                        args[0].int32().unwrap_or(0)
                    } else {
                        0
                    };
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            if idx >= 0 && (idx as usize) < s.len() {
                                if let Some(ch) = s.chars().nth(idx as usize) {
                                    return Ok(number_to_value(ctx, ch as u32 as f64));
                                }
                            }
                        }
                    }
                    return Ok(number_to_value(ctx, f64::NAN));
                } else if marker == "__builtin_string_charCodeAt__" {
                    let idx = if args.len() >= 1 {
                        args[0].int32().unwrap_or(0)
                    } else {
                        0
                    };
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            if idx >= 0 && (idx as usize) < s.len() {
                                if let Some(ch) = s.chars().nth(idx as usize) {
                                    return Ok(number_to_value(ctx, ch as u32 as f64));
                                }
                            }
                        }
                    }
                    return Ok(number_to_value(ctx, f64::NAN));
                } else if marker == "__builtin_string_codePointAt__" {
                    let idx = if args.len() >= 1 {
                        args[0].int32().unwrap_or(0)
                    } else {
                        0
                    };
                    if let Some(str_bytes) = ctx.string_bytes(this_val) {
                        if let Ok(s) = core::str::from_utf8(str_bytes) {
                            if idx >= 0 && (idx as usize) < s.len() {
                                if let Some(ch) = s.chars().nth(idx as usize) {
                                    return Ok(number_to_value(ctx, ch as u32 as f64));
                                }
                            }
                        }
                    }
                    return Ok(number_to_value(ctx, f64::NAN));
                } else if marker == "__builtin_number_toFixed__" {
                    let digits = if args.is_empty() {
                        0
                    } else {
                        js_to_int32(ctx, args[0]).unwrap_or(0)
                    };
                    if digits < 0 || digits > 100 {
                        js_throw_error(ctx, JSObjectClassEnum::RangeError, "toFixed() digits out of range");
                        return Err(());
                    }
                    let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                    let s = format_fixed(n, digits);
                    return Ok(js_new_string(ctx, &s));
                } else if marker == "__builtin_number_toPrecision__" {
                    if args.is_empty() {
                        return Ok(js_to_string(ctx, this_val));
                    }
                    let precision = js_to_int32(ctx, args[0]).unwrap_or(0);
                    if precision < 1 || precision > 100 {
                        js_throw_error(ctx, JSObjectClassEnum::RangeError, "toPrecision() precision out of range");
                        return Err(());
                    }
                    let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                    let s = format_precision(n, precision);
                    return Ok(js_new_string(ctx, &s));
                } else if marker == "__builtin_number_toExponential__" {
                    let digits_opt = if args.is_empty() {
                        None
                    } else {
                        let d = js_to_int32(ctx, args[0]).unwrap_or(0);
                        if d < 0 || d > 100 {
                            js_throw_error(ctx, JSObjectClassEnum::RangeError, "toExponential() digits out of range");
                            return Err(());
                        }
                        Some(d)
                    };
                    let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                    let s = format_exponential(n, digits_opt);
                    return Ok(js_new_string(ctx, &s));
                } else if marker == "__builtin_number_toString__" {
                    if args.is_empty() || args[0] == Value::UNDEFINED {
                        return Ok(js_to_string(ctx, this_val));
                    }
                    let radix = js_to_int32(ctx, args[0]).unwrap_or(10);
                    if radix < 2 || radix > 36 {
                        js_throw_error(ctx, JSObjectClassEnum::RangeError, "toString() radix must be between 2 and 36");
                        return Err(());
                    }
                    let n = js_to_number(ctx, this_val).unwrap_or(f64::NAN);
                    if !n.is_finite() || radix == 10 {
                        return Ok(js_to_string(ctx, this_val));
                    }
                    let rounded = n.trunc();
                    if rounded.abs() > (i64::MAX as f64) {
                        return Ok(js_to_string(ctx, this_val));
                    }
                    let s = format_radix_int(rounded as i64, radix as u32);
                    return Ok(js_new_string(ctx, &s));
                } else if marker == "__builtin_regexp_test__" {
                    let input = if args.is_empty() {
                        String::new()
                    } else {
                        value_to_string(ctx, args[0])
                    };
                    let (pattern, flags) = regexp_parts(ctx, this_val).unwrap_or_default();
                    let (re, _) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                    return Ok(Value::new_bool(re.is_match(&input).unwrap_or(false)));
                } else if marker == "__builtin_regexp_exec__" {
                    let input_val = if args.is_empty() {
                        js_new_string(ctx, "")
                    } else {
                        coerce_to_string_value(ctx, args[0])
                    };
                    let input = value_to_string(ctx, input_val);
                    let (pattern, flags) = regexp_parts(ctx, this_val).unwrap_or_default();
                    let (re, _) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                    if let Ok(Some(caps)) = re.captures(&input) {
                        let arr = js_new_array(ctx, caps.len() as i32);
                        for i in 0..caps.len() {
                            if let Some(m) = caps.get(i) {
                                let mv = js_new_string(ctx, m.as_str());
                                js_set_property_uint32(ctx, arr, i as u32, mv);
                            } else {
                                js_set_property_uint32(ctx, arr, i as u32, Value::UNDEFINED);
                            }
                        }
                        let idx = caps.get(0).map(|m| m.start() as i32).unwrap_or(0);
                        let _ = js_set_property_str(ctx, arr, "index", Value::from_int32(idx));
                        let _ = js_set_property_str(ctx, arr, "input", input_val);
                        return Ok(arr);
                    }
                    return Ok(Value::NULL);
                } else if marker == "__builtin_parseInt__" {
                    if args.len() >= 1 {
                        if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                if let Ok(n) = s.trim().parse::<i32>() {
                                    return Ok(Value::from_int32(n));
                                }
                                return Ok(number_to_value(ctx, f64::NAN));
                            }
                        } else if let Some(n) = args[0].int32() {
                            return Ok(Value::from_int32(n));
                        }
                    }
                    return Ok(number_to_value(ctx, f64::NAN));
                } else if marker == "__builtin_parseFloat__" {
                    if args.len() >= 1 {
                        if let Some(str_bytes) = ctx.string_bytes(args[0]) {
                            if let Ok(s) = core::str::from_utf8(str_bytes) {
                                let trimmed = s.trim_start();
                                if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
                                    return Ok(number_to_value(ctx, 0.0));
                                }
                                if let Ok(n) = trimmed.parse::<f64>() {
                                    return Ok(number_to_value(ctx, n));
                                }
                                return Ok(number_to_value(ctx, f64::NAN));
                            }
                        } else if let Ok(n) = js_to_number(ctx, args[0]) {
                            return Ok(number_to_value(ctx, n));
                        }
                    }
                    return Ok(number_to_value(ctx, f64::NAN));
                } else if marker == "__builtin_Number_isInteger__" {
                    if args.len() == 1 {
                        if args[0].is_number() {
                            return Ok(Value::TRUE);
                        }
                        if let Some(f) = ctx.float_value(args[0]) {
                            return Ok(Value::new_bool(f.is_finite() && f.fract() == 0.0));
                        }
                    }
                    return Ok(Value::FALSE);
                } else if marker == "__builtin_Number_isNaN__" {
                    if args.len() == 1 {
                        if let Some(f) = ctx.float_value(args[0]) {
                            return Ok(Value::new_bool(f.is_nan()));
                        }
                        return Ok(Value::FALSE);
                    }
                    return Ok(Value::FALSE);
                } else if marker == "__builtin_Number_isFinite__" {
                    if args.len() == 1 {
                        if args[0].is_number() {
                            return Ok(Value::TRUE);
                        }
                        if let Some(f) = ctx.float_value(args[0]) {
                            return Ok(Value::new_bool(f.is_finite()));
                        }
                    }
                    return Ok(Value::FALSE);
                } else if marker == "__builtin_Number_isSafeInteger__" {
                    if args.len() == 1 {
                        let max_safe = 9007199254740991.0_f64;
                        let is_safe = if let Some(n) = args[0].int32() {
                            (n as f64).abs() <= max_safe
                        } else if let Some(f) = ctx.float_value(args[0]) {
                            f.is_finite() && f.fract() == 0.0 && f.abs() <= max_safe
                        } else {
                            false
                        };
                        return Ok(Value::new_bool(is_safe));
                    }
                    return Ok(Value::FALSE);
                } else if marker == "__builtin_Math_floor__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(Value::from_int32(n.floor() as i32));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_ceil__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(Value::from_int32(n.ceil() as i32));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_round__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(Value::from_int32(n.round() as i32));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_abs__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.abs()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_max__" {
                    if !args.is_empty() {
                        let mut max = f64::NEG_INFINITY;
                        for arg in args {
                            if let Ok(n) = js_to_number(ctx, *arg) {
                                if n > max {
                                    max = n;
                                }
                            }
                        }
                        return Ok(number_to_value(ctx, max));
                    }
                    return Ok(number_to_value(ctx, f64::NEG_INFINITY));
                } else if marker == "__builtin_Math_min__" {
                    if !args.is_empty() {
                        let mut min = f64::INFINITY;
                        for arg in args {
                            if let Ok(n) = js_to_number(ctx, *arg) {
                                if n < min {
                                    min = n;
                                }
                            }
                        }
                        return Ok(number_to_value(ctx, min));
                    }
                    return Ok(number_to_value(ctx, f64::INFINITY));
                } else if marker == "__builtin_Math_sqrt__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.sqrt()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_pow__" {
                    if args.len() == 2 {
                        let base = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        let exp = js_to_number(ctx, args[1]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, base.powf(exp)));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_sin__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.sin()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_cos__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.cos()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_tan__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.tan()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_asin__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.asin()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_acos__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.acos()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_atan__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.atan()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_atan2__" {
                    if args.len() == 2 {
                        let y = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        let x = js_to_number(ctx, args[1]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, y.atan2(x)));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_exp__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.exp()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_log__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.ln()));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_log2__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.ln() / core::f64::consts::LN_2));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_log10__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        return Ok(number_to_value(ctx, n.ln() / core::f64::consts::LN_10));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_fround__" {
                    if args.len() == 1 {
                        let n = js_to_number(ctx, args[0]).map_err(|_| ())?;
                        let f = n as f32;
                        return Ok(number_to_value(ctx, f as f64));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_imul__" {
                    if args.len() == 2 {
                        let a = js_to_int32(ctx, args[0]).unwrap_or(0);
                        let b = js_to_int32(ctx, args[1]).unwrap_or(0);
                        return Ok(Value::from_int32(a.wrapping_mul(b)));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_Math_clz32__" {
                    if args.len() == 1 {
                        let n = js_to_uint32(ctx, args[0]).unwrap_or(0);
                        return Ok(Value::from_int32(n.leading_zeros() as i32));
                    }
                    return Ok(Value::UNDEFINED);
                // Array methods
                } else if marker == "__builtin_array_push__" {
                    for arg in args {
                        js_array_push(ctx, this_val, *arg);
                    }
                    let len = js_get_property_str(ctx, this_val, "length");
                    return Ok(len);
                } else if marker == "__builtin_array_pop__" {
                    return Ok(js_array_pop(ctx, this_val));
                } else if marker == "__builtin_array_join__" {
                    let separator = if args.len() >= 1 && args[0] != Value::UNDEFINED {
                        if let Some(bytes) = ctx.string_bytes(args[0]) {
                            core::str::from_utf8(bytes).unwrap_or(",").to_string()
                        } else if let Some(n) = args[0].int32() {
                            n.to_string()
                        } else {
                            ",".to_string()
                        }
                    } else {
                        ",".to_string()
                    };
                    let len_val = js_get_property_str(ctx, this_val, "length");
                    let len = len_val.int32().unwrap_or(0).max(0) as u32;
                    let mut result = String::new();
                    for i in 0..len {
                        if i > 0 {
                            result.push_str(&separator);
                        }
                        let elem = js_get_property_uint32(ctx, this_val, i);
                        if elem.is_undefined() || elem.is_null() {
                            continue;
                        }
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
                    return Ok(js_new_string(ctx, &result));
                } else if marker == "__builtin_typedarray_toString__" {
                    if let Some(ptr) = get_typedarray_data(ctx, this_val) {
                        let data = unsafe { &*ptr };
                        let mut out = String::new();
                        for i in 0..data.length {
                            if i > 0 {
                                out.push(',');
                            }
                            let v = typed_array_get_element(ctx, this_val, i as u32);
                            let s_val = js_to_string(ctx, v);
                            if let Some(bytes) = ctx.string_bytes(s_val) {
                                out.push_str(core::str::from_utf8(bytes).unwrap_or(""));
                            }
                        }
                        return Ok(js_new_string(ctx, &out));
                    }
                    return Ok(js_new_string(ctx, ""));
                } else if marker == "__builtin_typedarray_set__" {
                    if args.len() >= 1 {
                        let offset = if args.len() >= 2 {
                            js_to_int32(ctx, args[1]).unwrap_or(0).max(0) as usize
                        } else {
                            0
                        };
                        if let Some(ptr) = get_typedarray_data(ctx, this_val) {
                            let data = unsafe { &*ptr };
                            let len = data.length;
                            let mut idx = 0usize;
                            if let Some(src_class) = ctx.object_class_id(args[0]) {
                                if typed_array_kind_from_class_id(src_class).is_some() {
                                    let src_len = js_get_property_str(ctx, args[0], "length").int32().unwrap_or(0).max(0) as usize;
                                    while idx < src_len && offset + idx < len {
                                        let v = typed_array_get_element(ctx, args[0], idx as u32);
                                        typed_array_set_element(ctx, this_val, (offset + idx) as u32, v);
                                        idx += 1;
                                    }
                                } else if src_class == JSObjectClassEnum::Array as u32 {
                                    let src_len = js_get_property_str(ctx, args[0], "length").int32().unwrap_or(0).max(0) as usize;
                                    while idx < src_len && offset + idx < len {
                                        let v = js_get_property_uint32(ctx, args[0], idx as u32);
                                        typed_array_set_element(ctx, this_val, (offset + idx) as u32, v);
                                        idx += 1;
                                    }
                                }
                            }
                        }
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_typedarray_subarray__" {
                    if let Some(ptr) = get_typedarray_data(ctx, this_val) {
                        let data = unsafe { &*ptr };
                        let len = data.length as i32;
                        let mut begin = if args.len() >= 1 { js_to_int32(ctx, args[0]).unwrap_or(0) } else { 0 };
                        let mut end = if args.len() >= 2 { js_to_int32(ctx, args[1]).unwrap_or(len) } else { len };
                        if begin < 0 { begin = (len + begin).max(0); }
                        if end < 0 { end = (len + end).max(0); }
                        if begin > len { begin = len; }
                        if end > len { end = len; }
                        if end < begin { end = begin; }
                        let new_len = (end - begin) as usize;
                        let offset = data.offset + (begin as usize) * data.kind.elem_size();
                        if let Some(class_enum) = typed_array_class_enum_from_id(ctx.object_class_id(this_val).unwrap_or(0)) {
                            return Ok(create_typed_array(ctx, class_enum, data.buffer, offset, new_len));
                        }
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_array_slice__" {
                    let len_val = js_get_property_str(ctx, this_val, "length");
                    let len = len_val.int32().unwrap_or(0);
                    let start = if args.len() > 0 {
                        if let Some(s) = args[0].int32() {
                            let mut s = s;
                            if s < 0 { s += len; if s < 0 { s = 0; } }
                            s.min(len)
                        } else { len }
                    } else { len };
                    let final_idx = if args.len() > 1 {
                        if let Some(e) = args[1].int32() {
                            let mut e = e;
                            if e < 0 { e += len; if e < 0 { e = 0; } }
                            e.min(len)
                        } else { len }
                    } else { len };
                    let slice_len = (final_idx - start).max(0);
                    let arr = js_new_array(ctx, slice_len);
                    let mut idx = 0u32;
                    for i in start..final_idx {
                        let elem = js_get_property_uint32(ctx, this_val, i as u32);
                        js_set_property_uint32(ctx, arr, idx, elem);
                        idx += 1;
                    }
                    return Ok(arr);
                } else if marker == "__builtin_array_splice__" {
                    // Ported from mquickjs.c:14478-14548 js_array_splice
                    let len_val = js_get_property_str(ctx, this_val, "length");
                    let len = len_val.int32().unwrap_or(0);

                    let start = if args.len() > 0 {
                        if let Some(s) = args[0].int32() {
                            if s < 0 { (len + s).max(0) } else { s.min(len) }
                        } else { 0 }
                    } else { 0 };

                    let del_count = if args.len() > 1 {
                        if let Some(d) = args[1].int32() {
                            d.max(0).min(len - start)
                        } else { len - start }
                    } else if args.len() == 1 { len - start } else { 0 };

                    let items: Vec<JSValue> = if args.len() > 2 { args[2..].to_vec() } else { Vec::new() };
                    let item_count = items.len() as i32;

                    let result = js_new_array(ctx, del_count);
                    for i in 0..del_count {
                        let elem = js_get_property_uint32(ctx, this_val, (start + i) as u32);
                        js_set_property_uint32(ctx, result, i as u32, elem);
                    }

                    let new_len = len + item_count - del_count;
                    if item_count != del_count {
                        if item_count < del_count {
                            // Shrinking - shift left
                            for i in (start + item_count)..new_len {
                                let elem = js_get_property_uint32(ctx, this_val, (i + del_count - item_count) as u32);
                                js_set_property_uint32(ctx, this_val, i as u32, elem);
                            }
                        } else {
                            // Growing - first expand array by pushing
                            let extra = item_count - del_count;
                            for _ in 0..extra {
                                js_array_push(ctx, this_val, Value::UNDEFINED);
                            }
                            // Now shift elements right
                            for i in ((start + item_count)..new_len).rev() {
                                let elem = js_get_property_uint32(ctx, this_val, (i - extra) as u32);
                                js_set_property_uint32(ctx, this_val, i as u32, elem);
                            }
                        }
                    }

                    for (i, item) in items.into_iter().enumerate() {
                        js_set_property_uint32(ctx, this_val, (start + i as i32) as u32, item);
                    }
                    js_set_property_str(ctx, this_val, "length", Value::from_int32(new_len));
                    return Ok(result);
                } else if marker == "__builtin_array_indexOf__" {
                    if args.len() >= 1 {
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0);
                        let search_val = args[0];
                        let mut start = if args.len() >= 2 {
                            js_to_int32(ctx, args[1]).unwrap_or(0)
                        } else {
                            0
                        };
                        if start < 0 {
                            start = (len + start).max(0);
                        }
                        if start < len {
                            for i in start..len {
                                let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                if elem.0 == search_val.0 {
                                    return Ok(Value::from_int32(i as i32));
                                }
                            }
                        }
                        return Ok(Value::from_int32(-1));
                    }
                } else if marker == "__builtin_array_lastIndexOf__" {
                    if args.len() >= 1 {
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0);
                        let search_val = args[0];
                        let mut start = if args.len() >= 2 {
                            js_to_int32(ctx, args[1]).unwrap_or(len - 1)
                        } else {
                            len - 1
                        };
                        if start >= len {
                            start = len - 1;
                        }
                        if start < 0 {
                            start = len + start;
                        }
                        if start >= 0 {
                            let mut i = start;
                            loop {
                                let elem = js_get_property_uint32(ctx, this_val, i as u32);
                                if elem.0 == search_val.0 {
                                    return Ok(Value::from_int32(i as i32));
                                }
                                if i == 0 { break; }
                                i -= 1;
                            }
                        }
                        return Ok(Value::from_int32(-1));
                    }
                } else if marker == "__builtin_array_forEach__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        for i in 0..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [elem, idx_val, this_val];
                            call_closure(ctx, callback, &call_args);
                        }
                        return Ok(Value::UNDEFINED);
                    }
                } else if marker == "__builtin_array_map__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        let result = js_new_array(ctx, len as i32);
                        for i in 0..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [elem, idx_val, this_val];
                            if let Some(mapped) = call_closure(ctx, callback, &call_args) {
                                js_set_property_uint32(ctx, result, i, mapped);
                            }
                        }
                        return Ok(result);
                    }
                } else if marker == "__builtin_array_filter__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        let result = js_new_array(ctx, 0);
                        let mut result_idx = 0u32;
                        for i in 0..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [elem, idx_val, this_val];
                            if let Some(res) = call_closure(ctx, callback, &call_args) {
                                if is_truthy(ctx, res) {
                                    js_set_property_uint32(ctx, result, result_idx, elem);
                                    result_idx += 1;
                                }
                            }
                        }
                        return Ok(result);
                    }
                } else if marker == "__builtin_array_reduce__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        let mut accumulator = if args.len() >= 2 {
                            args[1]
                        } else if len > 0 {
                            js_get_property_uint32(ctx, this_val, 0)
                        } else {
                            return Ok(Value::UNDEFINED);
                        };
                        let start_idx = if args.len() >= 2 { 0 } else { 1 };
                        for i in start_idx..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [accumulator, elem, idx_val, this_val];
                            if let Some(res) = call_closure(ctx, callback, &call_args) {
                                accumulator = res;
                            }
                        }
                        return Ok(accumulator);
                    }
                } else if marker == "__builtin_array_reduceRight__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as i32;
                        if len <= 0 {
                            return Ok(Value::UNDEFINED);
                        }
                        let mut accumulator = if args.len() >= 2 {
                            args[1]
                        } else {
                            js_get_property_uint32(ctx, this_val, (len - 1) as u32)
                        };
                        let mut i = if args.len() >= 2 { len - 1 } else { len - 2 };
                        while i >= 0 {
                            let elem = js_get_property_uint32(ctx, this_val, i as u32);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [accumulator, elem, idx_val, this_val];
                            if let Some(res) = call_closure(ctx, callback, &call_args) {
                                accumulator = res;
                            }
                            if i == 0 { break; }
                            i -= 1;
                        }
                        return Ok(accumulator);
                    }
                } else if marker == "__builtin_array_every__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        for i in 0..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [elem, idx_val, this_val];
                            if let Some(res) = call_closure(ctx, callback, &call_args) {
                                if !is_truthy(ctx, res) {
                                    return Ok(Value::FALSE);
                                }
                            }
                        }
                        return Ok(Value::TRUE);
                    }
                } else if marker == "__builtin_array_some__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        for i in 0..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [elem, idx_val, this_val];
                            if let Some(res) = call_closure(ctx, callback, &call_args) {
                                if is_truthy(ctx, res) {
                                    return Ok(Value::TRUE);
                                }
                            }
                        }
                        return Ok(Value::FALSE);
                    }
                } else if marker == "__builtin_array_find__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        for i in 0..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [elem, idx_val, this_val];
                            if let Some(res) = call_closure(ctx, callback, &call_args) {
                                if is_truthy(ctx, res) {
                                    return Ok(elem);
                                }
                            }
                        }
                        return Ok(Value::UNDEFINED);
                    }
                } else if marker == "__builtin_array_findIndex__" {
                    if args.len() >= 1 {
                        let callback = args[0];
                        let len_val = js_get_property_str(ctx, this_val, "length");
                        let len = len_val.int32().unwrap_or(0) as u32;
                        for i in 0..len {
                            let elem = js_get_property_uint32(ctx, this_val, i);
                            let idx_val = Value::from_int32(i as i32);
                            let call_args = [elem, idx_val, this_val];
                            if let Some(res) = call_closure(ctx, callback, &call_args) {
                                if is_truthy(ctx, res) {
                                    return Ok(Value::from_int32(i as i32));
                                }
                            }
                        }
                        return Ok(Value::from_int32(-1));
                    }
                }
            }
        }

        // Check if it's a closure (custom function)
        let closure_marker = js_get_property_str(ctx, method, "__closure__");
        if closure_marker == Value::TRUE {
            if let Some(val) = call_closure(ctx, method, args) {
                return Ok(val);
            }
        }

        Err(())
    }

    fn parse_primary(&mut self) -> Result<JSValue, ()> {
        self.skip_ws();
        if let Some(b'(') = self.peek() {
            let start = self.pos;
            let mut depth = 0i32;
            let mut in_string = false;
            let mut string_delim = 0u8;
            let bytes = self.input;
            let mut i = self.pos;
            while i < bytes.len() {
                let b = bytes[i];
                if in_string {
                    if b == string_delim {
                        in_string = false;
                    } else if b == b'\\' && i + 1 < bytes.len() {
                        i += 1;
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
                if b == b'(' {
                    depth += 1;
                } else if b == b')' {
                    depth -= 1;
                    if depth == 0 {
                        let inner = &bytes[start + 1..i];
                        let inner_str = core::str::from_utf8(inner).map_err(|_| ())?;
                        self.pos = i + 1;
                        let ctx = unsafe { &mut *self.ctx };
                        let val = eval_expr(ctx, inner_str.trim()).ok_or(())?;
                        return Ok(val);
                    }
                }
                i += 1;
            }
            return Err(());
        }
        if self.peek() == Some(b'/') {
            return self.parse_regex_literal();
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

    fn parse_regex_literal(&mut self) -> Result<JSValue, ()> {
        self.pos += 1; // skip '/'
        let mut pattern = Vec::new();
        let mut escaped = false;
        let mut closed = false;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if escaped {
                pattern.push(b);
                escaped = false;
                continue;
            }
            if b == b'\\' {
                pattern.push(b);
                escaped = true;
                continue;
            }
            if b == b'/' {
                closed = true;
                break;
            }
            pattern.push(b);
        }
        if !closed {
            return Err(());
        }
        let mut flags = Vec::new();
        while let Some(b) = self.peek() {
            if b.is_ascii_alphabetic() {
                flags.push(b);
                self.pos += 1;
            } else {
                break;
            }
        }
        let pattern_str = core::str::from_utf8(&pattern).map_err(|_| ())?;
        let flags_str = core::str::from_utf8(&flags).map_err(|_| ())?;
        let ctx = unsafe { &mut *self.ctx };
        let val = js_new_regexp(ctx, pattern_str, flags_str);
        if val.is_exception() {
            return Err(());
        }
        Ok(val)
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
        if name == "globalThis" {
            let global = js_get_global_object(ctx);
            let val = js_get_property_str(ctx, global, "globalThis");
            if val.is_undefined() && !ctx.has_property_str(global, b"globalThis") {
                return Ok(global);
            }
            return Ok(val);
        }
        if let Some(val) = eval_value(ctx, name) {
            return Ok(val);
        }
        Err(())
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
        let mut neg = false;
        if self.peek() == Some(b'-') {
            neg = true;
            self.pos += 1;
        }
        if self.peek() == Some(b'0') {
            if let Some(next) = self.input.get(self.pos + 1).copied() {
                let (radix, is_prefix) = match next {
                    b'x' | b'X' => (16, true),
                    b'o' | b'O' => (8, true),
                    b'b' | b'B' => (2, true),
                    _ => (10, false),
                };
                if is_prefix {
                    self.pos += 2;
                    let start_digits = self.pos;
                    while let Some(b) = self.peek() {
                        let ok = match radix {
                            16 => b.is_ascii_hexdigit(),
                            8 => matches!(b, b'0'..=b'7'),
                            2 => matches!(b, b'0' | b'1'),
                            _ => b.is_ascii_digit(),
                        };
                        if ok {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    if self.pos == start_digits {
                        return Err(());
                    }
                    let slice = &self.input[start_digits..self.pos];
                    let s = core::str::from_utf8(slice).map_err(|_| ())?;
                    let v = u64::from_str_radix(s, radix).map_err(|_| ())? as f64;
                    return Ok(if neg { -v } else { v });
                }
            }
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
        let left_prim = if ctx.object_class_id(left).is_some() {
            js_to_primitive(ctx, left, true).unwrap_or(left)
        } else {
            left
        };
        let right_prim = if ctx.object_class_id(right).is_some() {
            js_to_primitive(ctx, right, true).unwrap_or(right)
        } else {
            right
        };

        let left_is_string = ctx.string_bytes(left_prim).is_some();
        let right_is_string = ctx.string_bytes(right_prim).is_some();
        if left_is_string || right_is_string {
            let ls = js_to_string(ctx, left_prim);
            let rs = js_to_string(ctx, right_prim);
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
        let ln = js_to_number(ctx, left_prim).map_err(|_| ())?;
        let rn = js_to_number(ctx, right_prim).map_err(|_| ())?;
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
                    if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb < rb
                    } else {
                        let ln = js_to_number(ctx, left).map_err(|_| ())?;
                        let rn = js_to_number(ctx, right).map_err(|_| ())?;
                        ln < rn
                    }
                }
                b'>' => {
                    if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb > rb
                    } else {
                        let ln = js_to_number(ctx, left).map_err(|_| ())?;
                        let rn = js_to_number(ctx, right).map_err(|_| ())?;
                        ln > rn
                    }
                }
                _ => return Err(()),
            }
        } else if op.len() == 2 {
            match (op[0], op[1]) {
                (b'<', b'=') => {
                    if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb <= rb
                    } else {
                        let ln = js_to_number(ctx, left).map_err(|_| ())?;
                        let rn = js_to_number(ctx, right).map_err(|_| ())?;
                        ln <= rn
                    }
                }
                (b'>', b'=') => {
                    if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb >= rb
                    } else {
                        let ln = js_to_number(ctx, left).map_err(|_| ())?;
                        let rn = js_to_number(ctx, right).map_err(|_| ())?;
                        ln >= rn
                    }
                }
                (b'=', b'=') => {
                    // Loose equality - JS == does type coercion
                    // null == undefined is true in JS
                    if (left.is_null() && right.is_undefined()) || (left.is_undefined() && right.is_null()) {
                        true
                    } else if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb == rb
                    } else
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
                    // Loose inequality
                    // null != undefined is false in JS
                    if (left.is_null() && right.is_undefined()) || (left.is_undefined() && right.is_null()) {
                        false
                    } else if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb != rb
                    } else
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
                    if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb == rb
                    } else
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
                    if let (Some(lb), Some(rb)) = (ctx.string_bytes(left), ctx.string_bytes(right)) {
                        lb != rb
                    } else
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

    fn starts_with_keyword(&self, kw: &[u8]) -> bool {
        let slice = &self.input[self.pos..];
        if !slice.starts_with(kw) {
            return false;
        }
        let next = slice.get(kw.len()).copied();
        let is_ident_char = |b: u8| -> bool {
            (b'A'..=b'Z').contains(&b)
                || (b'a'..=b'z').contains(&b)
                || (b'0'..=b'9').contains(&b)
                || b == b'_'
        };
        !next.map(is_ident_char).unwrap_or(false)
    }

    fn typeof_value(&self, val: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let type_str = if val.is_bool() {
            "boolean"
        } else if js_is_number(ctx, val) != 0 {
            "number"
        } else if js_is_string(ctx, val) != 0 {
            "string"
        } else if val.is_undefined() {
            "undefined"
        } else if val.is_null() {
            "object"
        } else if js_is_function(ctx, val) != 0 {
            "function"
        } else if val.is_ptr() {
            "object"
        } else {
            "undefined"
        };
        Ok(js_new_string(ctx, type_str))
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
