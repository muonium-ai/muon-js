//! Expression evaluation and program execution module.
//! 
//! This module contains the core evaluation functions that execute JavaScript code.
//! It handles variable declarations, assignments, operators, control flow, and built-in methods.

use crate::api::*;
use crate::types::*;
use crate::value::*;
use crate::helpers::*;
use crate::parser::{create_function, extract_braces, extract_paren, parse_identifier};

const BUILTIN_DISPATCH: [(&str, &str); 30] = [
    ("Array", "__builtin_Array__"),
    ("ArrayBuffer", "__builtin_ArrayBuffer__"),
    ("Date", "__builtin_Date__"),
    ("Error", "__builtin_Error__"),
    ("Float32Array", "__builtin_Float32Array__"),
    ("Float64Array", "__builtin_Float64Array__"),
    ("Function", "__builtin_Function__"),
    ("Int16Array", "__builtin_Int16Array__"),
    ("Int32Array", "__builtin_Int32Array__"),
    ("Int8Array", "__builtin_Int8Array__"),
    ("JSON", "__builtin_JSON__"),
    ("Math", "__builtin_Math__"),
    ("Number", "__builtin_Number__"),
    ("Object", "__builtin_Object__"),
    ("RangeError", "__builtin_RangeError__"),
    ("ReferenceError", "__builtin_ReferenceError__"),
    ("RegExp", "__builtin_RegExp__"),
    ("String", "__builtin_String__"),
    ("SyntaxError", "__builtin_SyntaxError__"),
    ("TypeError", "__builtin_TypeError__"),
    ("Uint16Array", "__builtin_Uint16Array__"),
    ("Uint32Array", "__builtin_Uint32Array__"),
    ("Uint8Array", "__builtin_Uint8Array__"),
    ("Uint8ClampedArray", "__builtin_Uint8ClampedArray__"),
    ("console", "__builtin_console__"),
    ("eval", "__builtin_eval__"),
    ("isFinite", "__builtin_isFinite__"),
    ("isNaN", "__builtin_isNaN__"),
    ("parseFloat", "__builtin_parseFloat__"),
    ("parseInt", "__builtin_parseInt__"),
];

fn lookup_builtin_dispatch(name: &str) -> Option<(&'static str, &'static str)> {
    BUILTIN_DISPATCH
        .binary_search_by_key(&name, |(builtin_name, _)| *builtin_name)
        .ok()
        .map(|idx| BUILTIN_DISPATCH[idx])
}

fn find_arrow_top_level(src: &str) -> Option<usize> {
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
            return Some(i);
        }
        i += 1;
    }
    None
}

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
    // Handle `this` keyword - look it up in the current environment
    if s == "this" {
        if let Some((_, val)) = ctx.resolve_binding("this") {
            return Some(val);
        }
        // If not bound, `this` is undefined in strict mode or global in sloppy mode
        return Some(js_get_global_object(ctx));
    }
    if let Some(pos) = find_arrow_top_level(s) {
        let (left, right) = s.split_at(pos);
        let params_src = left.trim();
        let body_src = right[2..].trim();
        let params: Vec<String> = if params_src.starts_with('(') {
            let (inside, rest) = extract_paren(params_src)?;
            if !rest.trim().is_empty() {
                return None;
            }
            crate::parser::parse_parameter_list(inside)?
        } else if is_identifier(params_src) {
            vec![params_src.to_string()]
        } else if params_src.is_empty() {
            Vec::new()
        } else {
            return None;
        };
        let body = if body_src.starts_with('{') {
            let (block, rest) = extract_braces(body_src)?;
            if !rest.trim().is_empty() {
                return None;
            }
            block.to_string()
        } else {
            format!("return {};", body_src)
        };
        let func = create_function(ctx, &params, &body)?;
        return Some(func);
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
        let params = crate::parser::parse_parameter_list(params_str)?;
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
    if let Some((builtin_name, marker)) = lookup_builtin_dispatch(s) {
        return Some(builtin_or_global(builtin_name, marker));
    }
    if s == "globalThis" {
        let val = js_get_property_str(ctx, global, "globalThis");
        if val.is_undefined() && !ctx.has_property_str(global, b"globalThis") {
            return Some(global);
        }
        return Some(val);
    }
    if s == "NaN" {
        return Some(number_to_value(ctx, f64::NAN));
    }
    if s == "Infinity" {
        return Some(number_to_value(ctx, f64::INFINITY));
    }
    if is_simple_string_literal(s) {
        let inner = &s[1..s.len() - 1];
        let unescaped = unescape_string_literal(inner);
        return Some(js_new_string(ctx, &unescaped));
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

pub(crate) fn unescape_string_literal(src: &str) -> String {
    let mut units: Vec<u16> = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut buf = [0u16; 2];
            let slice = ch.encode_utf16(&mut buf);
            units.extend_from_slice(slice);
            continue;
        }
        match chars.next() {
            Some('n') => units.push('\n' as u16),
            Some('r') => units.push('\r' as u16),
            Some('t') => units.push('\t' as u16),
            Some('b') => units.push(0x0008),
            Some('f') => units.push(0x000C),
            Some('v') => units.push(0x000B),
            Some('\\') => units.push('\\' as u16),
            Some('"') => units.push('"' as u16),
            Some('\'') => units.push('\'' as u16),
            Some('x') => {
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(h) = chars.next() {
                        hex.push(h);
                    }
                }
                if let Ok(v) = u16::from_str_radix(&hex, 16) {
                    units.push(v);
                }
            }
            Some('u') => {
                if let Some('{') = chars.peek().copied() {
                    let _ = chars.next();
                    let mut hex = String::new();
                    while let Some(h) = chars.next() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                    if let Ok(v) = u32::from_str_radix(&hex, 16) {
                        if v <= 0xFFFF {
                            units.push(v as u16);
                        } else {
                            let v = v - 0x10000;
                            let high = 0xD800 + ((v >> 10) as u16);
                            let low = 0xDC00 + ((v & 0x3FF) as u16);
                            units.push(high);
                            units.push(low);
                        }
                    }
                } else {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        if let Some(h) = chars.next() {
                            hex.push(h);
                        }
                    }
                    if let Ok(v) = u16::from_str_radix(&hex, 16) {
                        units.push(v);
                    }
                }
            }
            Some(other) => units.push(other as u16),
            None => break,
        }
    }
    utf16_units_to_string_preserve_surrogates(&units)
}

pub(crate) fn utf16_units_to_string_preserve_surrogates(units: &[u16]) -> String {
    let mut out = String::new();
    for &u in units {
        if (0xD800..=0xDFFF).contains(&u) {
            let mapped = (u as u32) + 0x800;
            if let Some(ch) = char::from_u32(mapped) {
                out.push(ch);
            }
        } else if let Some(ch) = char::from_u32(u as u32) {
            out.push(ch);
        }
    }
    out
}

/// Evaluate an expression (handles assignments, operators, method calls, etc.)
/// 
/// This is the main workhorse function that evaluates JavaScript expressions.
/// Due to its size (~2600 lines with built-in method handlers), it remains in api.rs
/// and is re-exported here for use by other modules.
pub use crate::api::eval_expr;

/// Strip line (`//`) and block (`/* */`) comments while preserving strings and length.
/// Returns an error offset if an unterminated block comment is found.
pub fn strip_comments_checked(src: &str) -> Result<String, usize> {
    let bytes = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    let mut in_string = false;
    let mut string_delim = 0u8;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b);
            if b == b'\\' {
                if i + 1 < bytes.len() {
                    out.push(bytes[i + 1]);
                    i += 2;
                    continue;
                }
            } else if b == string_delim {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' || b == b'"' {
            in_string = true;
            string_delim = b;
            out.push(b);
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            if next == b'/' {
                // Line comment: replace comment with spaces, keep newline.
                out.push(b' ');
                out.push(b' ');
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    out.push(b' ');
                    i += 1;
                }
                if i < bytes.len() && bytes[i] == b'\n' {
                    out.push(b'\n');
                    i += 1;
                }
                continue;
            }
            if next == b'*' {
                // Block comment: replace content with spaces, preserve newlines.
                let start = i;
                out.push(b' ');
                out.push(b' ');
                i += 2;
                let mut closed = false;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        out.push(b' ');
                        out.push(b' ');
                        i += 2;
                        closed = true;
                        break;
                    }
                    if bytes[i] == b'\n' {
                        out.push(b'\n');
                    } else {
                        out.push(b' ');
                    }
                    i += 1;
                }
                if !closed {
                    return Err(start);
                }
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8(out).map_err(|_| 0)
}

/// Strip comments without error reporting.
#[allow(dead_code)]
pub fn strip_comments(src: &str) -> String {
    strip_comments_checked(src).unwrap_or_else(|_| src.to_string())
}

/// Join single-line control flow without braces to keep statements together.
pub fn normalize_line_continuations(src: &str) -> String {
    fn line_ends_with_operator(line: &str) -> bool {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return false;
        }
        let bytes = trimmed.as_bytes();
        let len = bytes.len();
        let last = bytes[len - 1];
        if last == b'+' || last == b'-' {
            if len >= 2 && bytes[len - 2] == last {
                return false; // ++ or -- should not force continuation
            }
            return true;
        }
        matches!(
            last,
            b'*' | b'/' | b'%' | b'&' | b'|' | b'^' | b'<' | b'>' | b'=' | b'?' | b':' | b',' | b'.'
        )
    }

    let mut out = String::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        let mut current = line.to_string();
        loop {
            let trimmed = current.trim_start();
            let is_control = trimmed.starts_with("if ")
                || trimmed.starts_with("if(")
                || trimmed.starts_with("else")
                || trimmed.starts_with("function ")
                || trimmed.starts_with("while ")
                || trimmed.starts_with("while(")
                || trimmed.starts_with("for ")
                || trimmed.starts_with("for(")
                || trimmed.starts_with("try")
                || trimmed.starts_with("catch");
            let has_inline_body = if trimmed.starts_with("if")
                || trimmed.starts_with("while")
                || trimmed.starts_with("for")
            {
                if let Some(start) = trimmed.find('(') {
                    let mut depth = 0i32;
                    let mut end_pos = None;
                    for (idx, ch) in trimmed[start..].char_indices() {
                        match ch {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end_pos = Some(start + idx);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(end) = end_pos {
                        !trimmed[end + 1..].trim().is_empty()
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else if trimmed.starts_with("else") {
                trimmed != "else"
            } else if trimmed.starts_with("do") {
                trimmed != "do"
            } else {
                false
            };

            if is_control && !trimmed.contains('{') && !trimmed.trim_end().ends_with(';') && !has_inline_body {
                if let Some(next) = lines.next() {
                    current.push(' ');
                    current.push_str(next.trim_start());
                    continue;
                }
            }
            if trimmed.contains("function") && trimmed.trim_end().ends_with(')') {
                if let Some(next) = lines.peek() {
                    if next.trim_start().starts_with('{') {
                        let next = lines.next().unwrap();
                        current.push(' ');
                        current.push_str(next.trim_start());
                        continue;
                    }
                }
            }
            if line_ends_with_operator(&current) {
                if let Some(next) = lines.next() {
                    current.push(' ');
                    current.push_str(next.trim_start());
                    continue;
                }
            }
            break;
        }
        out.push_str(&current);
        out.push('\n');
    }
    out
}

/// Check if a value is truthy in JavaScript semantics
pub fn is_truthy(ctx: &mut JSContextImpl, val: JSValue) -> bool {
    if val.is_bool() {
        return val == Value::TRUE;
    }
    if let Some(n) = val.int32() {
        return n != 0;
    }
    if val.is_null() || val.is_undefined() {
        return false;
    }
    if let Some(f) = ctx.float_value(val) {
        return f != 0.0 && !f.is_nan();
    }
    if js_is_string(ctx, val) != 0 {
        if let Some(bytes) = ctx.string_bytes(val) {
            return !bytes.is_empty();
        }
        return false;
    }
    true
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
        let val = eval_expr(ctx, item)?;
        let res = js_set_property_uint32(ctx, arr, idx as u32, val);
        if res.is_exception() {
            return None;
        }
    }
    Some(arr)
}

/// Evaluate an object literal: {a: 1, b: 2}
/// Also handles getter/setter syntax: {get x() { return b; }, set x(v) { b = v; }}
/// And method shorthand: {f(v) { return v + 1 }}
pub fn eval_object_literal(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let inner = src.trim();
    let inner = &inner[1..inner.len().saturating_sub(1)];
    let entries = split_top_level(inner)?;
    let obj = js_new_object(ctx);
    if obj.is_exception() {
        return None;
    }
    for entry in entries {
        let entry = entry.trim();

        // Check for getter: get propertyName() { ... }
        if entry.starts_with("get ") {
            let rest = entry[4..].trim_start();
            // Find the property name and the function definition
            if let Some(paren_pos) = rest.find('(') {
                let prop_name = rest[..paren_pos].trim();
                // Make sure it's not "get: value" (property named "get")
                if !prop_name.is_empty() && is_identifier(prop_name) {
                    // Build a function from the rest: () { ... }
                    let func_src = &rest[paren_pos..];
                    if let Some((_params_str, after_params)) = extract_paren(func_src) {
                        let after_params = after_params.trim_start();
                        if after_params.starts_with('{') {
                            if let Some((body, _tail)) = extract_braces(after_params) {
                                // Create the getter function (no parameters)
                                let func = create_function(ctx, &[], body)?;
                                // Store as __get__propertyName
                                let getter_key = format!("__get__{}", prop_name);
                                let res = js_set_property_str(ctx, obj, &getter_key, func);
                                if res.is_exception() {
                                    return None;
                                }
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Check for setter: set propertyName(v) { ... }
        if entry.starts_with("set ") {
            let rest = entry[4..].trim_start();
            // Find the property name and the function definition
            if let Some(paren_pos) = rest.find('(') {
                let prop_name = rest[..paren_pos].trim();
                // Make sure it's not "set: value" (property named "set")
                if !prop_name.is_empty() && is_identifier(prop_name) {
                    // Build a function from the rest: (v) { ... }
                    let func_src = &rest[paren_pos..];
                    if let Some((params_str, after_params)) = extract_paren(func_src) {
                        let after_params = after_params.trim_start();
                        if after_params.starts_with('{') {
                            if let Some((body, _tail)) = extract_braces(after_params) {
                                // Parse parameters
                                let param_list = split_top_level(params_str)?;
                                let mut params = Vec::new();
                                for p in param_list {
                                    let p = p.trim();
                                    if !p.is_empty() {
                                        params.push(p.to_string());
                                    }
                                }
                                // Create the setter function
                                let func = create_function(ctx, &params, body)?;
                                // Store as __set__propertyName
                                let setter_key = format!("__set__{}", prop_name);
                                let res = js_set_property_str(ctx, obj, &setter_key, func);
                                if res.is_exception() {
                                    return None;
                                }
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Check for method shorthand: methodName(args) { ... }
        // Look for pattern: identifier(params) { body }
        if let Some(paren_pos) = entry.find('(') {
            let potential_name = entry[..paren_pos].trim();
            // Check it's an identifier and not a getter/setter property (those have ":")
            if is_identifier(potential_name) && !entry.contains(':') {
                let func_src = &entry[paren_pos..];
                if let Some((params_str, after_params)) = extract_paren(func_src) {
                    let after_params = after_params.trim_start();
                    if after_params.starts_with('{') {
                        if let Some((body, tail)) = extract_braces(after_params) {
                            if tail.trim().is_empty() {
                                // Parse parameters
                                let param_list = split_top_level(params_str)?;
                                let mut params = Vec::new();
                                for p in param_list {
                                    let p = p.trim();
                                    if !p.is_empty() {
                                        params.push(p.to_string());
                                    }
                                }
                                // Create the method function
                                let func = create_function(ctx, &params, body)?;
                                let res = js_set_property_str(ctx, obj, potential_name, func);
                                if res.is_exception() {
                                    return None;
                                }
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // Regular property: key: value
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

        // Handle __proto__ specially - set the prototype of the object
        if key_str == "__proto__" {
            ctx.set_object_proto(obj, val);
            continue;
        }

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
    let s = src;
    if s.trim().is_empty() {
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
            let p = part.trim();
            let rest = s[i + 1..].trim_start();
            if rest.starts_with("else") {
                continue;
            }
            if b == b'\n' {
                if p == "else" || p == "do" {
                    continue;
                }
                let looks_like_control = (p.starts_with("if ") || p.starts_with("if(")
                    || p.starts_with("while ") || p.starts_with("while(")
                    || p.starts_with("for ") || p.starts_with("for("))
                    && p.ends_with(')');
                if looks_like_control {
                    continue;
                }
            }
            if !p.is_empty() {
                out.push(p);
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

/// Split source into statements with starting offsets (preserving leading whitespace).
#[allow(dead_code)]
pub fn split_statements_with_offsets(src: &str) -> Option<Vec<(usize, &str)>> {
    let s = src;
    if s.trim().is_empty() {
        return Some(Vec::new());
    }
    let bytes = s.as_bytes();
    let mut out: Vec<(usize, &str)> = Vec::new();
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
        if b == b'\'' || b == b'"' {
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
                let part = &s[start..=i];
                let trimmed = part.trim();
                if trimmed.starts_with("if ") || trimmed.starts_with("if(")
                    || trimmed.starts_with("while ") || trimmed.starts_with("while(")
                    || trimmed.starts_with("for ") || trimmed.starts_with("for(")
                    || trimmed.starts_with("function ") {
                    if !trimmed.is_empty() {
                        out.push((start, part));
                    }
                    start = i + 1;
                }
            }
            continue;
        }
        if (b == b';' || b == b'\n') && depth == 0 {
            let part = &s[start..i];
            let p = part.trim();
            let rest = s[i + 1..].trim_start();
            if rest.starts_with("else") {
                continue;
            }
            if b == b'\n' {
                if p == "else" || p == "do" {
                    continue;
                }
                let looks_like_control = (p.starts_with("if ") || p.starts_with("if(")
                    || p.starts_with("while ") || p.starts_with("while(")
                    || p.starts_with("for ") || p.starts_with("for("))
                    && p.ends_with(')');
                if looks_like_control {
                    continue;
                }
            }
            if !p.is_empty() {
                out.push((start, part));
            }
            start = i + 1;
        }
    }
    if depth != 0 {
        return None;
    }
    let part = &s[start..];
    if !part.trim().is_empty() {
        out.push((start, part));
    }
    Some(out)
}
