//! Statement and expression parsing module.
//!
//! This module contains all the parsing logic for JavaScript control flow statements,
//! function declarations, and expression decomposition.

use crate::api::*;
use crate::types::*;
use crate::value::*;
use crate::evals::*;

/// LValue key type for property access
pub enum LValueKey {
    Index(u32),
    Name(String),
}

/// Parse function declaration: "function name(params) { body }"
/// Stores the function in the global object.
pub fn parse_function_declaration(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
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
pub fn extract_braces(s: &str) -> Option<(&str, &str)> {
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

/// Extract content within parentheses ( )
pub fn extract_paren(src: &str) -> Option<(&str, &str)> {
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

/// Extract content within brackets [ ]
pub fn extract_bracket(src: &str) -> Option<(&str, &str)> {
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

/// Parse if statement: "if (condition) { block } else { block }"
pub fn parse_if_statement(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
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
pub fn parse_while_loop(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
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

/// Find " in " keyword in for loop header
pub fn find_for_in_keyword(header: &str) -> Option<usize> {
    let bytes = header.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;

    let mut i = 0;
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
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }

        if depth == 0 && i + 4 <= bytes.len() {
            if &bytes[i..i + 4] == b" in " {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Find " of " keyword in for loop header
pub fn find_for_of_keyword(header: &str) -> Option<usize> {
    let bytes = header.as_bytes();
    for i in 0..header.len().saturating_sub(4) {
        if bytes[i] == b' ' && bytes[i+1] == b'o' && bytes[i+2] == b'f' && bytes[i+3] == b' ' {
            return Some(i + 1);
        }
    }
    None
}

/// Parse for...of loop
pub fn parse_for_of_loop(ctx: &mut JSContextImpl, header: &str, of_pos: usize, after_header: &str) -> Option<JSValue> {
    let var_part = header[..of_pos].trim();
    let iter_expr = header[of_pos + 3..].trim();

    let var_name = if var_part.starts_with("var ") {
        var_part[4..].trim()
    } else if var_part.starts_with("const ") {
        var_part[6..].trim()
    } else if var_part.starts_with("let ") {
        var_part[4..].trim()
    } else {
        var_part
    };

    if !after_header.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_header)?;

    let iter_val = eval_expr(ctx, iter_expr)?;

    if let Some(class_id) = ctx.object_class_id(iter_val) {
        if class_id == JSObjectClassEnum::Array as u32 {
            if let Some(len_val) = ctx.get_property_str(iter_val, b"length") {
                if let Some(len) = len_val.int32() {
                    let mut last = Value::UNDEFINED;
                    let global = js_get_global_object(ctx);

                    for i in 0..len {
                        let elem = js_get_property_uint32(ctx, iter_val, i as u32);
                        js_set_property_str(ctx, global, var_name, elem);
                        last = eval_function_body(ctx, body)?;

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
                                return Some(last);
                            }
                            crate::context::LoopControl::None => {}
                        }
                    }

                    return Some(last);
                }
            }
        }
    }

    Some(Value::UNDEFINED)
}

/// Parse for...in loop
pub fn parse_for_in_loop(ctx: &mut JSContextImpl, header: &str, in_pos: usize, after_header: &str) -> Option<JSValue> {
    let var_part = header[..in_pos].trim();
    let obj_expr = header[in_pos + 4..].trim();

    let var_name = if var_part.starts_with("var ") {
        var_part[4..].trim()
    } else {
        var_part
    };

    if !after_header.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_header)?;

    let obj_val = eval_expr(ctx, obj_expr)?;
    let keys = get_object_keys(ctx, obj_val)?;

    let mut last = Value::UNDEFINED;
    let global = js_get_global_object(ctx);

    for key in keys {
        let key_val = js_new_string(ctx, &key);
        js_set_property_str(ctx, global, var_name, key_val);

        last = eval_function_body(ctx, body)?;

        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
            crate::context::LoopControl::Return => {
                break;
            }
            crate::context::LoopControl::None => {}
        }
    }

    Some(last)
}

/// Get object keys (handles arrays and objects)
pub fn get_object_keys(ctx: &mut JSContextImpl, obj: JSValue) -> Option<Vec<String>> {
    if let Some(class_id) = ctx.object_class_id(obj) {
        if class_id == JSObjectClassEnum::Array as u32 {
            let len_val = js_get_property_str(ctx, obj, "length");
            let len = len_val.int32().unwrap_or(0);
            let mut keys = Vec::new();
            for i in 0..len {
                keys.push(i.to_string());
            }
            return Some(keys);
        }
    }

    ctx.object_keys(obj)
}

/// Parse for loop: "for (init; condition; update) { block }"
pub fn parse_for_loop(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    let rest = if s.starts_with("for ") {
        &s[4..]
    } else if s.starts_with("for(") {
        &s[3..]
    } else {
        return None;
    };

    let rest = rest.trim_start();

    if !rest.starts_with('(') {
        return None;
    }
    let (header, after_header) = extract_paren(rest)?;
    let after_header = after_header.trim_start();

    if let Some(in_pos) = find_for_in_keyword(header) {
        return parse_for_in_loop(ctx, header, in_pos, after_header);
    }

    if let Some(of_pos) = find_for_of_keyword(header) {
        return parse_for_of_loop(ctx, header, of_pos, after_header);
    }

    let parts: Vec<&str> = header.split(';').collect();
    if parts.len() != 3 {
        return None;
    }
    let init = parts[0].trim();
    let condition = parts[1].trim();
    let update = parts[2].trim();
    
    if !after_header.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_header)?;
    
    if !init.is_empty() {
        eval_expr(ctx, init)?;
    }
    
    let mut last = Value::UNDEFINED;
    loop {
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
        
        last = eval_function_body(ctx, body)?;
        
        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
            crate::context::LoopControl::Return => {
                break;
            }
            crate::context::LoopControl::None => {}
        }
        
        if !update.is_empty() {
            eval_expr(ctx, update)?;
        }
    }
    
    Some(last)
}

/// Parse do...while loop
pub fn parse_do_while_loop(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    let rest = if s.starts_with("do ") {
        &s[3..]
    } else if s.starts_with("do{") {
        &s[2..]
    } else {
        return None;
    };

    let rest = rest.trim_start();

    if !rest.starts_with('{') {
        return None;
    }
    let (body, after_body) = extract_braces(rest)?;
    let after_body = after_body.trim_start();

    if !after_body.starts_with("while") {
        return None;
    }
    let after_while = after_body[5..].trim_start();

    if !after_while.starts_with('(') {
        return None;
    }
    let (condition, _) = extract_paren(after_while)?;

    let mut last = eval_function_body(ctx, body)?;
    loop {
        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
            crate::context::LoopControl::Return => {
                break;
            }
            crate::context::LoopControl::None => {}
        }

        let cond_val = eval_expr(ctx, condition)?;
        if !is_truthy(cond_val) {
            break;
        }
        last = eval_function_body(ctx, body)?;
    }

    Some(last)
}

/// Parse try/catch/finally
pub fn parse_try_catch(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();

    let rest = if s.starts_with("try ") {
        &s[4..]
    } else if s.starts_with("try{") {
        &s[3..]
    } else {
        return None;
    };

    let rest = rest.trim();

    let (try_body, after_try) = extract_braces(rest)?;
    let after_try = after_try.trim();

    let mut catch_param: Option<&str> = None;
    let mut catch_body: Option<&str> = None;
    let mut finally_body: Option<&str> = None;
    let mut remaining = after_try;

    if remaining.starts_with("catch") {
        let after_catch = remaining[5..].trim();

        let body_start = if after_catch.starts_with('(') {
            if let Some(paren_end) = after_catch.find(')') {
                catch_param = Some(after_catch[1..paren_end].trim());
                after_catch[paren_end + 1..].trim()
            } else {
                return None;
            }
        } else {
            after_catch
        };

        let (cb, after_catch_body) = extract_braces(body_start)?;
        catch_body = Some(cb);
        remaining = after_catch_body.trim();
    }

    if remaining.starts_with("finally") {
        let after_finally = remaining[7..].trim();
        let (fb, _) = extract_braces(after_finally)?;
        finally_body = Some(fb);
    }

    if catch_body.is_none() && finally_body.is_none() {
        return None;
    }

    let try_result = eval_function_body(ctx, try_body);

    let mut result = match try_result {
        Some(val) if val.is_exception() => {
            if let Some(body) = catch_body {
                let exception_val = ctx.get_exception();
                ctx.set_exception(Value::UNDEFINED);

                if let Some(param) = catch_param {
                    let global = ctx.global_object();
                    js_set_property_str(ctx, global, param, exception_val);
                }

                match eval_function_body(ctx, body) {
                    Some(v) => v,
                    None => return None,
                }
            } else {
                val
            }
        }
        Some(val) => val,
        None => {
            if let Some(body) = catch_body {
                ctx.set_exception(Value::UNDEFINED);

                if let Some(param) = catch_param {
                    let global = ctx.global_object();
                    js_set_property_str(ctx, global, param, Value::UNDEFINED);
                }

                match eval_function_body(ctx, body) {
                    Some(v) => v,
                    None => return None,
                }
            } else {
                return None;
            }
        }
    };

    if let Some(body) = finally_body {
        let saved_exception = if result.is_exception() {
            ctx.get_exception()
        } else {
            Value::UNDEFINED
        };

        match eval_function_body(ctx, body) {
            Some(finally_val) => {
                if finally_val.is_exception() {
                    result = finally_val;
                } else if saved_exception != Value::UNDEFINED {
                    ctx.set_exception(saved_exception);
                }
            }
            None => return None,
        }
    }

    Some(result)
}

/// Parse switch statement
pub fn parse_switch_statement(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let s = src.trim();
    let rest = if s.starts_with("switch ") {
        &s[7..]
    } else if s.starts_with("switch(") {
        &s[6..]
    } else {
        return None;
    };

    let rest = rest.trim_start();

    if !rest.starts_with('(') {
        return None;
    }
    let (switch_expr, after_expr) = extract_paren(rest)?;
    let after_expr = after_expr.trim_start();

    let switch_val = eval_expr(ctx, switch_expr)?;

    if !after_expr.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_expr)?;

    let cases = parse_switch_cases(body)?;

    let mut matched = false;
    let mut last = Value::UNDEFINED;
    let mut found_default: Option<&str> = None;

    for (case_expr, case_body) in &cases {
        if case_expr.is_none() {
            found_default = Some(case_body);
            if matched {
                last = eval_function_body(ctx, case_body)?;
                if ctx.get_loop_control() == crate::context::LoopControl::Break {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    break;
                }
            }
            continue;
        }

        if matched {
            last = eval_function_body(ctx, case_body)?;
            if ctx.get_loop_control() == crate::context::LoopControl::Break {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            continue;
        }

        let case_val = eval_expr(ctx, case_expr.unwrap())?;

        if switch_val.0 == case_val.0 {
            matched = true;
            last = eval_function_body(ctx, case_body)?;
            if ctx.get_loop_control() == crate::context::LoopControl::Break {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
        }
    }

    if !matched {
        if let Some(default_body) = found_default {
            last = eval_function_body(ctx, default_body)?;
            if ctx.get_loop_control() == crate::context::LoopControl::Break {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
        }
    }

    Some(last)
}

/// Parse switch cases
pub fn parse_switch_cases(body: &str) -> Option<Vec<(Option<&str>, &str)>> {
    let mut cases = Vec::new();
    let mut rest = body.trim();

    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }

        if rest.starts_with("case ") {
            let after_case = &rest[5..];
            let colon_pos = find_case_colon(after_case)?;
            let case_expr = after_case[..colon_pos].trim();
            rest = &after_case[colon_pos + 1..];

            let body_end = find_case_body_end(rest);
            let case_body = &rest[..body_end];
            rest = &rest[body_end..];

            cases.push((Some(case_expr), case_body.trim()));
        } else if rest.starts_with("default") {
            let after_default = rest[7..].trim_start();
            if !after_default.starts_with(':') {
                return None;
            }
            rest = &after_default[1..];

            let body_end = find_case_body_end(rest);
            let case_body = &rest[..body_end];
            rest = &rest[body_end..];

            cases.push((None, case_body.trim()));
        } else {
            break;
        }
    }

    Some(cases)
}

/// Find colon in case expression
pub fn find_case_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;

    for i in 0..bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == string_delim {
                in_string = false;
            } else if b == b'\\' && i + 1 < bytes.len() {
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
            b':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Find end of case body
pub fn find_case_body_end(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut i = 0;

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
        match b {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    return i;
                }
                depth -= 1;
            }
            _ => {}
        }

        if depth == 0 {
            if bytes[i..].starts_with(b"case ") || bytes[i..].starts_with(b"default") {
                return i;
            }
        }
        i += 1;
    }
    bytes.len()
}

/// Parse lvalue for assignment (handles obj.prop and arr[idx])
pub fn parse_lvalue(ctx: &mut JSContextImpl, src: &str) -> Option<(JSValue, LValueKey)> {
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

/// Parse identifier from string
pub fn parse_identifier(src: &str) -> Option<(&str, &str)> {
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

/// Check if byte is valid identifier start
pub fn is_ident_start(b: u8) -> bool {
    (b'A'..=b'Z').contains(&b) || (b'a'..=b'z').contains(&b) || b == b'_'
}

/// Check if string is valid identifier
pub fn is_identifier(s: &str) -> bool {
    let (name, rest) = match parse_identifier(s) {
        Some(v) => v,
        None => return false,
    };
    !name.is_empty() && rest.trim().is_empty()
}

/// Split assignment: "lhs = rhs"
pub fn split_assignment(src: &str) -> Option<(&str, &str)> {
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

/// Split ternary: "cond ? true_val : false_val"
pub fn split_ternary(src: &str) -> Option<(&str, &str, &str)> {
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut question_pos = None;
    
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

/// Split expression into base and tail (for property access)
pub fn split_base_and_tail(src: &str) -> Option<(&str, &str)> {
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

/// Create a function object with parameters and body
pub fn create_function(ctx: &mut JSContextImpl, params: &[String], body: &str) -> Option<JSValue> {
    let params_str = params.join(",");
    let params_val = js_new_string(ctx, &params_str);
    
    let body_val = js_new_string(ctx, body);
    
    let func = js_new_object(ctx);
    
    js_set_property_str(ctx, func, "__params__", params_val);
    js_set_property_str(ctx, func, "__body__", body_val);
    js_set_property_str(ctx, func, "__closure__", Value::TRUE);

    let env = js_new_object(ctx);
    let global = js_get_global_object(ctx);
    if let Some(keys) = get_object_keys(ctx, global) {
        for key in keys {
            let val = js_get_property_str(ctx, global, &key);
            js_set_property_str(ctx, env, &key, val);
        }
    }
    js_set_property_str(ctx, func, "__env__", env);
    
    Some(func)
}

/// Call a closure with arguments
pub fn call_closure(ctx: &mut JSContextImpl, func: JSValue, args: &[JSValue]) -> Option<JSValue> {
    let params_val = js_get_property_str(ctx, func, "__params__");
    let body_val = js_get_property_str(ctx, func, "__body__");
    let env_val = js_get_property_str(ctx, func, "__env__");
    
    let params_bytes = ctx.string_bytes(params_val)?;
    let params_str = core::str::from_utf8(params_bytes).ok()?.to_string();
    let param_names: Vec<String> = if params_str.is_empty() {
        Vec::new()
    } else {
        params_str.split(',').map(|s| s.trim().to_string()).collect()
    };
    
    let body_bytes = ctx.string_bytes(body_val)?;
    let body_str = core::str::from_utf8(body_bytes).ok()?.to_string();
    
    let saved_global = js_get_global_object(ctx);
    let mut saved_env = Vec::new();
    if let Some(keys) = get_object_keys(ctx, env_val) {
        for key in keys {
            let had = ctx.has_property_str(saved_global, key.as_bytes());
            let old = if had {
                js_get_property_str(ctx, saved_global, &key)
            } else {
                Value::UNDEFINED
            };
            saved_env.push((key.clone(), had, old));
            let val = js_get_property_str(ctx, env_val, &key);
            js_set_property_str(ctx, saved_global, &key, val);
        }
    }

    let mut saved = Vec::with_capacity(param_names.len());
    for (i, param_name) in param_names.iter().enumerate() {
        let arg_val = args.get(i).copied().unwrap_or(Value::UNDEFINED);
        let had = ctx.has_property_str(saved_global, param_name.as_bytes());
        let old = if had {
            js_get_property_str(ctx, saved_global, param_name)
        } else {
            Value::UNDEFINED
        };
        saved.push((param_name.clone(), had, old));
        js_set_property_str(ctx, saved_global, param_name, arg_val);
    }
    
    let result = eval_function_body(ctx, &body_str);
    
    if ctx.get_loop_control() == crate::context::LoopControl::Return {
        ctx.set_loop_control(crate::context::LoopControl::None);
    }

    for (name, had, old) in saved {
        if had {
            js_set_property_str(ctx, saved_global, &name, old);
        } else {
            js_set_property_str(ctx, saved_global, &name, Value::UNDEFINED);
        }
    }

    for (name, had, old) in saved_env {
        if had {
            js_set_property_str(ctx, saved_global, &name, old);
        } else {
            js_set_property_str(ctx, saved_global, &name, Value::UNDEFINED);
        }
    }
    
    result
}

/// Check if value is truthy
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
