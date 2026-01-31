//! Expression evaluation and program execution module.
//! 
//! This module contains the core evaluation functions that execute JavaScript code.
//! It handles variable declarations, assignments, operators, control flow, and built-in methods.

use crate::api::*;
use crate::types::*;
use crate::value::*;
use crate::helpers::*;
use crate::parser::{create_function, extract_braces, extract_paren, parse_identifier};

/// Evaluate a simple value expression (literals, identifiers, etc.)
pub fn eval_value(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
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
    if s.starts_with("function") {
        let rest = s[8..].trim_start();
        let (name_opt, after_name) = if rest.starts_with('(') {
            (None, rest)
        } else {
            let (name, after) = parse_identifier(rest)?;
            (Some(name.to_string()), after.trim_start())
        };
        if !after_name.starts_with('(') {
            return None;
        }
        let (params_str, after_params) = extract_paren(after_name)?;
        let after_params = after_params.trim_start();
        if !after_params.starts_with('{') {
            return None;
        }
        let (body, tail) = extract_braces(after_params)?;
        if !tail.trim().is_empty() {
            return None;
        }
        let param_list = split_top_level(params_str)?;
        let mut params = Vec::new();
        for p in param_list {
            let p = p.trim();
            if !p.is_empty() {
                params.push(p.to_string());
            }
        }
        let func = create_function(ctx, &params, body)?;
        if let Some(name) = name_opt {
            let name_val = js_new_string(ctx, &name);
            js_set_property_str(ctx, func, "name", name_val);
        }
        return Some(func);
    }
    let global = js_get_global_object(ctx);
    if is_identifier(s) {
        if let Some((_, val)) = ctx.resolve_binding(s) {
            if val == Value::UNINITIALIZED {
                return Some(js_throw_error(
                    ctx,
                    JSObjectClassEnum::ReferenceError,
                    "cannot access before initialization",
                ));
            }
            return Some(val);
        }
    }
    let mut builtin_or_global = |name: &str, marker: &str| -> JSValue {
        let val = js_get_property_str(ctx, global, name);
        if val.is_undefined() && !ctx.has_property_str(global, name.as_bytes()) {
            js_new_string(ctx, marker)
        } else {
            val
        }
    };
    if s == "Math" {
        return Some(builtin_or_global("Math", "__builtin_Math__"));
    }
    if s == "Object" {
        return Some(builtin_or_global("Object", "__builtin_Object__"));
    }
    if s == "Array" {
        return Some(builtin_or_global("Array", "__builtin_Array__"));
    }
    if s == "JSON" {
        return Some(builtin_or_global("JSON", "__builtin_JSON__"));
    }
    if s == "Number" {
        return Some(builtin_or_global("Number", "__builtin_Number__"));
    }
    if s == "String" {
        return Some(builtin_or_global("String", "__builtin_String__"));
    }
    if s == "RegExp" {
        return Some(builtin_or_global("RegExp", "__builtin_RegExp__"));
    }
    if s == "Date" {
        return Some(builtin_or_global("Date", "__builtin_Date__"));
    }
    if s == "console" {
        return Some(builtin_or_global("console", "__builtin_console__"));
    }
    if s == "parseInt" {
        return Some(builtin_or_global("parseInt", "__builtin_parseInt__"));
    }
    if s == "parseFloat" {
        return Some(builtin_or_global("parseFloat", "__builtin_parseFloat__"));
    }
    if s == "eval" {
        return Some(builtin_or_global("eval", "__builtin_eval__"));
    }
    if s == "isNaN" {
        return Some(builtin_or_global("isNaN", "__builtin_isNaN__"));
    }
    if s == "isFinite" {
        return Some(builtin_or_global("isFinite", "__builtin_isFinite__"));
    }
    if s == "globalThis" {
        let val = js_get_property_str(ctx, global, "globalThis");
        if val.is_undefined() && !ctx.has_property_str(global, b"globalThis") {
            return Some(global);
        }
        return Some(val);
    }
    // Error constructors
    if s == "Error" {
        return Some(builtin_or_global("Error", "__builtin_Error__"));
    }
    if s == "TypeError" {
        return Some(builtin_or_global("TypeError", "__builtin_TypeError__"));
    }
    if s == "ReferenceError" {
        return Some(builtin_or_global("ReferenceError", "__builtin_ReferenceError__"));
    }
    if s == "SyntaxError" {
        return Some(builtin_or_global("SyntaxError", "__builtin_SyntaxError__"));
    }
    if s == "RangeError" {
        return Some(builtin_or_global("RangeError", "__builtin_RangeError__"));
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
        if let Ok(val) = crate::api::parse_arith_expr(ctx, s) {
            return Some(val);
        }
    }
    if let Ok(num) = crate::api::parse_numeric_expr(s) {
        return Some(number_to_value(ctx, num));
    }
    if s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        return eval_expr(ctx, inner);
    }
    if is_identifier(s) {
        let v = js_get_property_str(ctx, global, s);
        return Some(v);
    }
    None
}

/// Evaluate an expression (handles assignments, operators, method calls, etc.)
/// 
/// This is the main workhorse function that evaluates JavaScript expressions.
/// Due to its size (~2600 lines with built-in method handlers), it remains in api.rs
/// and is re-exported here for use by other modules.
pub use crate::api::eval_expr;

/// Check if a value is truthy in JavaScript semantics
pub fn is_truthy(val: JSValue) -> bool {
    if val.is_bool() {
        val == Value::TRUE
    } else if val.is_number() {
        if let Some(n) = val.int32() {
            n != 0
        } else {
            true
        }
    } else {
        !val.is_null() && !val.is_undefined()
    }
}

/// Evaluate an array literal: [1, 2, 3]
pub fn eval_array_literal(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
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

/// Evaluate an object literal: {a: 1, b: 2}
pub fn eval_object_literal(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
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

/// Split a comma-separated list at top level (respecting nesting)
pub fn split_top_level(src: &str) -> Option<Vec<&str>> {
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

/// Split source into statements (respecting control flow structures)
pub fn split_statements(src: &str) -> Option<Vec<&str>> {
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
            if depth == 0 && b == b'}' {
                let rest = s[i + 1..].trim_start();
                if rest.starts_with("else ") || rest.starts_with("else{") {
                    continue;
                }
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
