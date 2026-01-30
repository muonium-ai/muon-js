//! Helper utilities used across multiple modules

use crate::context::Context as JSContextImpl;
use crate::types::*;
use crate::value::Value;

/// Convert f64 to JSValue, using int32 for integers when possible
pub fn number_to_value(ctx: &mut JSContextImpl, n: f64) -> JSValue {
    if n.is_nan() || n.is_infinite() || n.fract() != 0.0 || n.abs() > i32::MAX as f64 {
        use crate::api::js_new_float64;
        js_new_float64(ctx, n)
    } else {
        Value::from_int32(n as i32)
    }
}

/// Check if a character is the start of an identifier
pub fn is_ident_start(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$')
}

/// Check if a string is a valid identifier
pub fn is_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    is_ident_start(bytes[0]) && bytes[1..].iter().all(|&b| is_ident_start(b) || matches!(b, b'0'..=b'9'))
}

/// Check if code point is a high surrogate (for UTF-16)
pub fn is_high_surrogate(code: u32) -> bool {
    (0xD800..=0xDBFF).contains(&code)
}

/// Check if code point is a low surrogate (for UTF-16)
pub fn is_low_surrogate(code: u32) -> bool {
    (0xDC00..=0xDFFF).contains(&code)
}

/// Check if a string contains arithmetic operators
pub fn contains_arith_op(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut depth = 0i32;

    for i in 0..bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == string_delim {
                in_string = false;
            }
            continue;
        }
        if b == b'\'' || b == b'"' {
            in_string = true;
            string_delim = b;
            continue;
        }
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            match b {
                b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'&' | b'|' | b'^' | b'~' => {
                    return true;
                }
                _ => {}
            }
        }
    }
    false
}

/// Check if a string is a simple string literal ("..." or '...')
pub fn is_simple_string_literal(src: &str) -> bool {
    let bytes = src.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    let first = bytes[0];
    let last = bytes[bytes.len() - 1];
    (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'')
}

/// Flatten an array recursively (for Array.flat)
pub fn flatten_array(ctx: &mut JSContextImpl, source: JSValue, target: JSValue, depth: i32) {
    use crate::api::{js_get_property_str, js_get_property_uint32, js_set_property_uint32};
    
    if depth <= 0 {
        // If depth is 0, just push the source as-is
        let len = js_get_property_str(ctx, target, "length");
        if let Some(idx) = len.int32() {
            js_set_property_uint32(ctx, target, idx as u32, source);
        }
        return;
    }
    
    // Check if source is an array
    if let Some(class_id) = ctx.object_class_id(source) {
        if class_id == JSObjectClassEnum::Array as u32 {
            // Get the length of source array
            if let Some(src_len_val) = ctx.get_property_str(source, b"length") {
                if let Some(src_len) = src_len_val.int32() {
                    // Iterate through source array
                    for i in 0..src_len {
                        let elem = js_get_property_uint32(ctx, source, i as u32);
                        
                        // Check if element is array and depth > 0
                        if depth > 0 {
                            if let Some(elem_class_id) = ctx.object_class_id(elem) {
                                if elem_class_id == JSObjectClassEnum::Array as u32 {
                                    // Recursively flatten
                                    flatten_array(ctx, elem, target, depth - 1);
                                    continue;
                                }
                            }
                        }
                        
                        // Not an array or depth is 0, just push it
                        let tgt_len = js_get_property_str(ctx, target, "length");
                        if let Some(idx) = tgt_len.int32() {
                            js_set_property_uint32(ctx, target, idx as u32, elem);
                        }
                    }
                }
            }
            return;
        }
    }
    
    // Not an array, push as-is
    let len = js_get_property_str(ctx, target, "length");
    if let Some(idx) = len.int32() {
        js_set_property_uint32(ctx, target, idx as u32, source);
    }
}
