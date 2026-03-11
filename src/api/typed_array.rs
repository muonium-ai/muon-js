//! Typed array (ArrayBuffer, Int8Array, Float64Array ...) support.

use crate::types::*;
use crate::value::Value;
use crate::helpers::number_to_value;
#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypedArrayKind {
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
    pub(crate) fn elem_size(self) -> usize {
        match self {
            TypedArrayKind::Uint8 | TypedArrayKind::Uint8Clamped | TypedArrayKind::Int8 => 1,
            TypedArrayKind::Int16 | TypedArrayKind::Uint16 => 2,
            TypedArrayKind::Int32 | TypedArrayKind::Uint32 | TypedArrayKind::Float32 => 4,
            TypedArrayKind::Float64 => 8,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ArrayBufferData {
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct TypedArrayData {
    pub(crate) buffer: JSValue,
    pub(crate) offset: usize,
    pub(crate) length: usize,
    pub(crate) kind: TypedArrayKind,
}

pub(super) fn typed_array_kind_from_class_id(class_id: u32) -> Option<TypedArrayKind> {
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

pub(super) fn typed_array_class_enum_from_id(class_id: u32) -> Option<JSObjectClassEnum> {
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

pub(super) fn get_arraybuffer_data(ctx: &mut JSContextImpl, val: JSValue) -> Option<*mut ArrayBufferData> {
    if ctx.object_class_id(val)? != JSObjectClassEnum::ArrayBuffer as u32 {
        return None;
    }
    let ptr = ctx.get_object_opaque(val) as *mut ArrayBufferData;
    if ptr.is_null() { None } else { Some(ptr) }
}

pub(super) fn get_typedarray_data(ctx: &mut JSContextImpl, val: JSValue) -> Option<*mut TypedArrayData> {
    let class_id = ctx.object_class_id(val)?;
    if typed_array_kind_from_class_id(class_id).is_none() {
        return None;
    }
    let ptr = ctx.get_object_opaque(val) as *mut TypedArrayData;
    if ptr.is_null() { None } else { Some(ptr) }
}

pub(super) fn clamp_u8_clamped(n: f64) -> u8 {
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

pub(super) fn typed_array_get_element(ctx: &mut JSContextImpl, obj: JSValue, idx: u32) -> JSValue {
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

pub(super) fn typed_array_set_element(ctx: &mut JSContextImpl, obj: JSValue, idx: u32, val: JSValue) -> bool {
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

pub(super) fn create_arraybuffer(ctx: &mut JSContextImpl, byte_len: usize) -> JSValue {
    let obj = js_new_object_class_user(ctx, JSObjectClassEnum::ArrayBuffer as i32);
    if obj.is_exception() {
        return obj;
    }
    let data = Box::new(ArrayBufferData { bytes: vec![0u8; byte_len] });
    ctx.set_object_opaque(obj, Box::into_raw(data) as *mut core::ffi::c_void);
    let _ = js_set_property_str(ctx, obj, "byteLength", Value::from_int32(byte_len as i32));
    obj
}

pub(super) fn create_typed_array(ctx: &mut JSContextImpl, class_id: JSObjectClassEnum, buffer: JSValue, offset: usize, length: usize) -> JSValue {
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

pub(super) fn build_typed_array_from_args(ctx: &mut JSContextImpl, class_id: JSObjectClassEnum, args: &[JSValue]) -> JSValue {
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

