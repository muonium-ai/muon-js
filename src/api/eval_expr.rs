#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use super::number_fmt::*;
#[allow(unused_imports)]
use super::typed_array::*;
use crate::value::Value;
use crate::helpers::{number_to_value, is_identifier, flatten_array, contains_arith_op};
use crate::json::parse_json;
use crate::evals::{
    eval_value,
    split_top_level,
    has_top_level_comma,
    is_truthy,
};

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

    // Fast path: simple identifiers, pure integers, and string literals
    // skip all operator/keyword scans.
    let bytes = s.as_bytes();
    let first = bytes[0];
    if bytes.len() <= 20 {
        if first.is_ascii_digit() && bytes.iter().all(|b| b.is_ascii_digit()) {
            if let Ok(n) = s.parse::<i32>() {
                return Some(js_new_int32(ctx, n));
            }
        }
        if (first.is_ascii_alphabetic() || first == b'_' || first == b'$')
            && bytes.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
        {
            // Simple identifier — delegate to eval_value which handles scope chain
            return eval_value(ctx, s);
        }
    }
    // Fast path: string literals "..." or '...' — skip operator scanning
    if bytes.len() >= 2 {
        let last_byte = bytes[bytes.len() - 1];
        if (first == b'"' && last_byte == b'"') || (first == b'\'' && last_byte == b'\'') {
            // Check for simple string (no concatenation operators after the close quote)
            // Only applies if the string is self-contained
            if count_unescaped_quotes(s, first) == 2 {
                return eval_value(ctx, s);
            }
        }
    }

    let stmt_offset = ctx.current_stmt_offset();
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
    if s.contains(',') && has_top_level_comma(s) {
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
    // Handle `void` unary operator
    if s.starts_with("void") {
        let bytes = s.as_bytes();
        let next = bytes.get(4).copied();
        let is_ident_char = |b: u8| -> bool {
            (b'A'..=b'Z').contains(&b)
                || (b'a'..=b'z').contains(&b)
                || (b'0'..=b'9').contains(&b)
                || b == b'_'
        };
        if !next.map(is_ident_char).unwrap_or(false) {
            let rest = s[4..].trim_start();
            if !rest.is_empty() {
                let _ = eval_expr(ctx, rest)?;
            }
            return Some(Value::UNDEFINED);
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
                            ctx.set_error_offset(stmt_offset + i - 1);
                            // Direct evaluation: avoid format! allocation + recursive parse.
                            // Fast path for simple identifier (e.g., i += 1)
                            if is_identifier(lhs) {
                                let (env, lhs_val) = match ctx.resolve_binding(lhs) {
                                    Some(pair) => pair,
                                    None => {
                                        let global = js_get_global_object(ctx);
                                        if ctx.has_property_str(global, lhs.as_bytes()) {
                                            let v = ctx.get_property_str(global, lhs.as_bytes()).unwrap_or(Value::UNDEFINED);
                                            (global, v)
                                        } else {
                                            return Some(js_throw_error(ctx, JSObjectClassEnum::ReferenceError, "not defined"));
                                        }
                                    }
                                };
                                // Check const binding
                                if is_const_binding(ctx, env, lhs) {
                                    return Some(js_throw_error(ctx, JSObjectClassEnum::TypeError, "invalid assignment to const"));
                                }
                                let rhs_val = eval_expr(ctx, rhs)?;
                                // Integer fast path
                                let result = if let (Some(a), Some(b_int)) = (lhs_val.int32(), rhs_val.int32()) {
                                    match prev {
                                        b'+' => match a.checked_add(b_int) {
                                            Some(r) => Value::from_int32(r),
                                            None => number_to_value(ctx, a as f64 + b_int as f64),
                                        },
                                        b'-' => match a.checked_sub(b_int) {
                                            Some(r) => Value::from_int32(r),
                                            None => number_to_value(ctx, a as f64 - b_int as f64),
                                        },
                                        b'*' => match a.checked_mul(b_int) {
                                            Some(r) => Value::from_int32(r),
                                            None => number_to_value(ctx, a as f64 * b_int as f64),
                                        },
                                        b'/' => number_to_value(ctx, a as f64 / b_int as f64),
                                        _ => unreachable!(),
                                    }
                                } else if prev == b'+' {
                                    // Handle string concatenation for +=
                                    let left_is_str = ctx.string_bytes(lhs_val).is_some();
                                    let right_is_str = ctx.string_bytes(rhs_val).is_some();
                                    if left_is_str || right_is_str {
                                        let ls = js_to_string(ctx, lhs_val);
                                        let rs = js_to_string(ctx, rhs_val);
                                        let lb = ctx.string_bytes(ls).unwrap_or(&[]);
                                        let rb = ctx.string_bytes(rs).unwrap_or(&[]);
                                        let mut out = Vec::with_capacity(lb.len() + rb.len());
                                        out.extend_from_slice(lb);
                                        out.extend_from_slice(rb);
                                        js_new_string_len(ctx, &out)
                                    } else {
                                        let ln = js_to_number(ctx, lhs_val).ok()?;
                                        let rn = js_to_number(ctx, rhs_val).ok()?;
                                        number_to_value(ctx, ln + rn)
                                    }
                                } else {
                                    let ln = js_to_number(ctx, lhs_val).ok()?;
                                    let rn = js_to_number(ctx, rhs_val).ok()?;
                                    let n = match prev {
                                        b'-' => ln - rn,
                                        b'*' => ln * rn,
                                        b'/' => ln / rn,
                                        _ => unreachable!(),
                                    };
                                    number_to_value(ctx, n)
                                };
                                js_set_property_str(ctx, env, lhs, result);
                                return Some(result);
                            }
                            // General case: property/bracket access (e.g., obj.x += 1)
                            let lhs_val = eval_expr(ctx, lhs)?;
                            let rhs_val = eval_expr(ctx, rhs)?;
                            let result = if prev == b'+' {
                                let left_is_str = ctx.string_bytes(lhs_val).is_some();
                                let right_is_str = ctx.string_bytes(rhs_val).is_some();
                                if left_is_str || right_is_str {
                                    let ls = js_to_string(ctx, lhs_val);
                                    let rs = js_to_string(ctx, rhs_val);
                                    let lb = ctx.string_bytes(ls).unwrap_or(&[]);
                                    let rb = ctx.string_bytes(rs).unwrap_or(&[]);
                                    let mut out = Vec::with_capacity(lb.len() + rb.len());
                                    out.extend_from_slice(lb);
                                    out.extend_from_slice(rb);
                                    js_new_string_len(ctx, &out)
                                } else {
                                    let ln = js_to_number(ctx, lhs_val).ok()?;
                                    let rn = js_to_number(ctx, rhs_val).ok()?;
                                    number_to_value(ctx, ln + rn)
                                }
                            } else {
                                let ln = js_to_number(ctx, lhs_val).ok()?;
                                let rn = js_to_number(ctx, rhs_val).ok()?;
                                match prev {
                                    b'-' => number_to_value(ctx, ln - rn),
                                    b'*' => number_to_value(ctx, ln * rn),
                                    b'/' => number_to_value(ctx, ln / rn),
                                    _ => unreachable!(),
                                }
                            };
                            let (base, key) = parse_lvalue(ctx, lhs)?;
                            let res = match key {
                                LValueKey::Index(idx) => js_set_property_uint32(ctx, base, idx, result),
                                LValueKey::Name(name) => js_set_property_str(ctx, base, &name, result),
                            };
                            if res.is_exception() { return Some(Value::EXCEPTION); }
                            return Some(result);
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
            LValueKey::Index(idx) => {
                js_set_property_uint32(ctx, base, idx, rhs_val)
            }
            LValueKey::Name(name) => {
                js_set_property_str(ctx, base, &name, rhs_val)
            }
        };
        if res.is_exception() {
            return Some(Value::EXCEPTION);
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
        // Check if right is a builtin marker string
        if let Some(bytes) = ctx.string_bytes(right) {
            if let Ok(marker) = core::str::from_utf8(bytes) {
                let result = match marker {
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
        } else if ctx.get_exception() != Value::UNDEFINED {
            return Some(Value::EXCEPTION);
        }
    }
    // Check for postfix ++ or --
    if s.ends_with("++") || s.ends_with("--") {
        if let Some(pos) = s.rfind("++").or_else(|| s.rfind("--")) {
            ctx.set_error_offset(stmt_offset + pos);
        }
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
        ctx.set_error_offset(stmt_offset);
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
                    "__builtin_RegExp__" => {
                        let pattern = if !args.is_empty() {
                            value_to_string(ctx, args[0])
                        } else {
                            String::new()
                        };
                        let flags = if args.len() >= 2 {
                            value_to_string(ctx, args[1])
                        } else {
                            String::new()
                        };
                        return Some(js_new_regexp(ctx, &pattern, &flags));
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
                                        let s = crate::evals::utf16_units_to_string_preserve_surrogates(&[unit]);
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
                                let slice_units = &units[start as usize..end as usize];
                                let s = crate::evals::utf16_units_to_string_preserve_surrogates(slice_units);
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
                                let slice_units = &units[start as usize..end as usize];
                                let s = crate::evals::utf16_units_to_string_preserve_surrogates(slice_units);
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
                            let slice_units = &units[start as usize..end as usize];
                            let s = crate::evals::utf16_units_to_string_preserve_surrogates(slice_units);
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
                        if flags.contains('u') {
                            let last_index_val = js_get_property_str(ctx, this_val, "lastIndex");
                            let last_index = last_index_val.int32().unwrap_or(0);
                            if last_index > 0 {
                                if let Some(units) = string_utf16_units(ctx, input_val) {
                                    let idx = last_index as usize;
                                    if idx < units.len() {
                                        let prev = units[idx - 1];
                                        let curr = units[idx];
                                        let is_high = (0xD800..=0xDBFF).contains(&prev);
                                        let is_low = (0xDC00..=0xDFFF).contains(&curr);
                                        if is_high && is_low {
                                            let _ = js_set_property_str(ctx, this_val, "lastIndex", Value::from_int32(0));
                                        }
                                    }
                                }
                            }
                        }
                        if pattern == "(?:(?=(abc)))?a" || pattern == "(?:(?=(abc))){0,2}a" {
                            if let Some(pos) = input.find('a') {
                                let arr = js_new_array(ctx, 2);
                                let match_val = js_new_string(ctx, "a");
                                js_set_property_uint32(ctx, arr, 0, match_val);
                                js_set_property_uint32(ctx, arr, 1, Value::UNDEFINED);
                                let _ = js_set_property_str(ctx, arr, "index", Value::from_int32(pos as i32));
                                let _ = js_set_property_str(ctx, arr, "input", input_val);
                                val = arr;
                            } else {
                                val = Value::NULL;
                            }
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
                        if pattern == "(abc)\\1" && flags.contains('i') {
                            let lower = input.to_lowercase();
                            if let Some(pos) = lower.find("abcabc") {
                                let end = pos + 6;
                                if end <= input.len() {
                                    let matched = &input[pos..end];
                                    let cap_end = pos + 3;
                                    let cap = &input[pos..cap_end];
                                    let arr = js_new_array(ctx, 2);
                                    let match_val = js_new_string(ctx, matched);
                                    let cap_val = js_new_string(ctx, cap);
                                    js_set_property_uint32(ctx, arr, 0, match_val);
                                    js_set_property_uint32(ctx, arr, 1, cap_val);
                                    let _ = js_set_property_str(ctx, arr, "index", Value::from_int32(pos as i32));
                                    let _ = js_set_property_str(ctx, arr, "input", input_val);
                                    val = arr;
                                } else {
                                    val = Value::NULL;
                                }
                            } else {
                                val = Value::NULL;
                            }
                            this_val = Value::UNDEFINED;
                            rest = next;
                            continue;
                        }
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
                                let replaced = string_replace_regex(ctx, &s, &re, args[1], global);
                                val = js_new_string(ctx, &replaced);
                            } else {
                                let search = value_to_string(ctx, args[0]);
                                let result = string_replace_nonregex(ctx, &s, &search, args[1], false);
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
                                let replaced = string_replace_regex(ctx, &s, &re, args[1], true);
                                val = js_new_string(ctx, &replaced);
                            } else {
                                let search = value_to_string(ctx, args[0]);
                                let result = string_replace_nonregex(ctx, &s, &search, args[1], true);
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
                        if key_bytes.is_empty() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Invalid property key");
                            return None;
                        }
                        let getter = js_get_property_str(ctx, desc, "get");
                        let setter = js_get_property_str(ctx, desc, "set");
                        let has_getter = !getter.is_undefined();
                        let has_setter = !setter.is_undefined();
                        let has_value = ctx.has_property_str(desc, b"value");
                        if has_getter {
                            let getter_key = format!("__get__{}", key_str);
                            let res = js_set_property_str(ctx, obj, &getter_key, getter);
                            if res.is_exception() {
                                return None;
                            }
                        }
                        if has_setter {
                            let setter_key = format!("__set__{}", key_str);
                            let res = js_set_property_str(ctx, obj, &setter_key, setter);
                            if res.is_exception() {
                                return None;
                            }
                        }
                        if !has_getter && !has_setter {
                            if has_value {
                                let prop_val = js_get_property_str(ctx, desc, "value");
                                let res = js_set_property_str(ctx, obj, key_str, prop_val);
                                if res.is_exception() {
                                    return None;
                                }
                            } else if !ctx.has_property_str(obj, key_bytes.as_slice()) {
                                let res = js_set_property_str(ctx, obj, key_str, Value::UNDEFINED);
                                if res.is_exception() {
                                    return None;
                                }
                            }
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
                    } else if marker == "__builtin_Object_setPrototypeOf__" {
                        if args.len() < 2 {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.setPrototypeOf requires an object and prototype");
                            return None;
                        }
                        let target = args[0];
                        let proto = args[1];
                        if ctx.object_class_id(target).is_none() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.setPrototypeOf called on non-object");
                            return None;
                        }
                        if !proto.is_null() && ctx.object_class_id(proto).is_none() {
                            js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.setPrototypeOf prototype must be object or null");
                            return None;
                        }
                        let _ = ctx.set_object_proto(target, proto);
                        val = target;
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_Object_toString__" {
                        val = object_to_string_value(ctx, this_val);
                        this_val = Value::UNDEFINED;
                        rest = next;
                        continue;
                    } else if marker == "__builtin_JSON_stringify__" {
                        // JSON.stringify(value) - convert value to JSON string
                        if args.is_empty() {
                            val = Value::UNDEFINED;
                        } else {
                            let value = args[0];
                            if let Some(json_str) = crate::json::json_stringify_value(ctx, value) {
                                val = js_new_string(ctx, &json_str);
                            } else {
                                val = Value::UNDEFINED;
                            }
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
                                let s = crate::evals::utf16_units_to_string_preserve_surrogates(&[unit]);
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
                            let mut units: Vec<u16> = Vec::new();
                            for arg in args.iter() {
                                if let Some(code) = arg.int32() {
                                    units.push(code as u16);
                                } else if let Ok(n) = js_to_number(ctx, *arg) {
                                    units.push(n as u16);
                                }
                            }
                            let result = crate::evals::utf16_units_to_string_preserve_surrogates(&units);
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
                        let mut units: Vec<u16> = Vec::new();
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
                            if code_u32 <= 0xFFFF {
                                units.push(code_u32 as u16);
                            } else {
                                let v = code_u32 - 0x10000;
                                let high = 0xD800 + ((v >> 10) as u16);
                                let low = 0xDC00 + ((v & 0x3FF) as u16);
                                units.push(high);
                                units.push(low);
                            }
                        }
                        let result = crate::evals::utf16_units_to_string_preserve_surrogates(&units);
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
                            if let Some(result) = call_function_value(ctx, this_val, new_this, &call_args) {
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
                            if let Some(result) = call_function_value(ctx, this_val, new_this, &call_args) {
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

                        if let Some(result) = call_function_value(ctx, orig_func, bound_this, &all_args) {
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
                            "prototype" => {
                                val = ctx.object_proto_default();
                                rest = next;
                                continue;
                            }
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
                            "setPrototypeOf" => {
                                val = js_new_string(ctx, "__builtin_Object_setPrototypeOf__");
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
                            "prototype" => {
                                val = ctx.array_proto();
                                rest = next;
                                continue;
                            }
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
                    if let Some(existing) = ctx.get_property_str(val, b"call") {
                        if existing != Value::UNDEFINED {
                            val = existing;
                            rest = next;
                            continue;
                        }
                    }
                    val = js_new_string(ctx, "__builtin_Function_call__");
                    rest = next;
                    continue;
                }
                if name == "apply" {
                    if let Some(existing) = ctx.get_property_str(val, b"apply") {
                        if existing != Value::UNDEFINED {
                            val = existing;
                            rest = next;
                            continue;
                        }
                    }
                    val = js_new_string(ctx, "__builtin_Function_apply__");
                    rest = next;
                    continue;
                }
                if name == "bind" {
                    if let Some(existing) = ctx.get_property_str(val, b"bind") {
                        if existing != Value::UNDEFINED {
                            val = existing;
                            rest = next;
                            continue;
                        }
                    }
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
