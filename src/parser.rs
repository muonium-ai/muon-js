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

pub fn eval_block(ctx: &mut JSContextImpl, body: &str) -> Option<JSValue> {
    let parent = ctx.current_env();
    let env = js_new_object(ctx);
    js_set_property_str(ctx, env, "__parent__", parent);
    js_set_property_str(ctx, env, "__block__", Value::TRUE);
    ctx.push_env(env);
    predeclare_block_bindings(ctx, body);
    let result = eval_function_body(ctx, body);
    ctx.pop_env();
    result
}

/// Execute a block from pre-parsed cached statements (avoids re-parsing).
fn eval_block_cached(ctx: &mut JSContextImpl, stmts: &[String]) -> Option<JSValue> {
    let parent = ctx.current_env();
    let env = js_new_object(ctx);
    js_set_property_str(ctx, env, "__parent__", parent);
    js_set_property_str(ctx, env, "__block__", Value::TRUE);
    ctx.push_env(env);
    predeclare_block_bindings_from_stmts(ctx, stmts);
    let result = eval_function_body_cached(ctx, stmts);
    ctx.pop_env();
    result
}

fn predeclare_block_bindings(ctx: &mut JSContextImpl, body: &str) {
    let stmts = match split_statements(body) {
        Some(s) => s,
        None => return,
    };
    predeclare_block_bindings_from_stmts(ctx, &stmts);
}

/// Pre-declare let/const bindings from already-parsed statement list.
fn predeclare_block_bindings_from_stmts(ctx: &mut JSContextImpl, stmts: &[impl AsRef<str>]) {
    let env = ctx.current_env();
    for stmt in stmts {
        let trimmed = stmt.as_ref().trim();
        let (kind, rest) = if trimmed.starts_with("let ") {
            ("let", trimmed[4..].trim())
        } else if trimmed.starts_with("const ") {
            ("const", trimmed[6..].trim())
        } else {
            continue;
        };
        let name = rest.split('=').next().unwrap_or("").trim();
        if is_identifier(name) {
            js_set_property_str(ctx, env, name, Value::UNINITIALIZED);
            if kind == "const" {
                mark_const_binding(ctx, env, name);
            }
        }
    }
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
    let params = parse_parameter_list(params_str)?;
    
    // Parse function body
    let after_params = after_params.trim_start();
    if !after_params.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_params)?;
    
    // Create a closure object
    let func = create_function(ctx, &params, body)?;
    
    // Store function in current environment
    let env = ctx.current_env();
    js_set_property_str(ctx, env, name, func);
    
    Some(func)
}

pub fn parse_parameter_list(params_str: &str) -> Option<Vec<String>> {
    let param_list = split_top_level(params_str)?;
    let mut params = Vec::with_capacity(param_list.len());
    for p in param_list {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            params.push(trimmed.to_owned());
        }
    }
    Some(params)
}

/// Parse labeled statement: "label: statement"
/// Returns None if not a labeled statement, Some(None) if parsing failed, Some(Some(val)) on success
pub fn parse_labeled_statement(ctx: &mut JSContextImpl, src: &str) -> Option<Option<JSValue>> {
    let s = src.trim();
    
    // Find colon after a label - but skip colons inside parentheses/brackets/braces/strings
    let bytes = s.as_bytes();
    let mut colon_pos = None;
    let mut paren_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut in_string = false;
    let mut string_delim = 0u8;
    
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if b == string_delim && (i == 0 || bytes[i - 1] != b'\\') {
                in_string = false;
            }
            continue;
        }
        if b == b'\'' || b == b'"' || b == b'`' {
            in_string = true;
            string_delim = b;
            continue;
        }
        match b {
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b'[' => bracket_depth += 1,
            b']' => bracket_depth = bracket_depth.saturating_sub(1),
            b'{' => brace_depth += 1,
            b'}' => brace_depth = brace_depth.saturating_sub(1),
            b':' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                colon_pos = Some(i);
                break;
            }
            _ => {}
        }
    }
    
    let colon_pos = colon_pos?;
    let label = &s[..colon_pos];
    
    // Verify label is a valid identifier (not part of ternary)
    let label = label.trim();
    if !is_identifier(label) {
        return None;
    }
    
    let statement = s[colon_pos + 1..].trim();
    
    // Check for block label: "label: { ... }"
    if statement.starts_with('{') {
        let (block, _) = extract_braces(statement)?;
        let result = eval_block(ctx, block);
        // Handle break label inside the block
        match ctx.get_loop_control() {
            crate::context::LoopControl::BreakLabel(ref l) if l == label => {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
            _ => {}
        }
        return Some(result);
    }
    
    // Check for labeled loop statements
    if statement.starts_with("for ") || statement.starts_with("for(") {
        return Some(parse_for_loop(ctx, statement, Some(label)));
    }
    if statement.starts_with("while ") || statement.starts_with("while(") {
        return Some(parse_while_loop(ctx, statement, Some(label)));
    }
    if statement.starts_with("do ") || statement.starts_with("do{") {
        return Some(parse_do_while_loop(ctx, statement, Some(label)));
    }
    
    // Other labeled statement (just execute it)
    let result = eval_function_body(ctx, statement);
    // Handle break label
    match ctx.get_loop_control() {
        crate::context::LoopControl::BreakLabel(ref l) if l == label => {
            ctx.set_loop_control(crate::context::LoopControl::None);
        }
        _ => {}
    }
    Some(result)
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

fn extract_statement(src: &str) -> Option<(&str, &str)> {
    let s = src.trim_start();
    if s.is_empty() {
        return None;
    }
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
                // Skip escaped char
                continue;
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
            b';' | b'\n' if depth == 0 => {
                let stmt = s[..i].trim();
                let rest = s[i + 1..].trim_start();
                return Some((stmt, rest));
            }
            _ => {}
        }
    }
    Some((s.trim(), ""))
}

/// Split a do...while statement into (body, while_part) at top-level.
fn split_do_while_body(src: &str) -> Option<(&str, &str)> {
    let s = src.trim_start();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let mut i = 0usize;
    while i + 5 <= bytes.len() {
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
        if b == b'\'' || b == b'"' || b == b'`' {
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
        if depth == 0 && i + 5 <= bytes.len() && &bytes[i..i + 5] == b"while" {
            let prev = if i == 0 { b' ' } else { bytes[i - 1] };
            let next = if i + 5 < bytes.len() { bytes[i + 5] } else { b' ' };
            let prev_is_ident = prev.is_ascii_alphanumeric() || prev == b'_';
            let next_is_ident = next.is_ascii_alphanumeric() || next == b'_';
            if !prev_is_ident && !next_is_ident {
                let body = s[..i].trim_end();
                let rest = s[i..].trim_start();
                return Some((body, rest));
            }
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
    
    // Parse then body (block or single statement)
    let (then_body, after_then, then_is_block) = if after_cond.starts_with('{') {
        let (block, after) = extract_braces(after_cond)?;
        (block, after.trim_start(), true)
    } else {
        let (stmt, after) = extract_statement(after_cond)?;
        (stmt, after.trim_start(), false)
    };

    // Parse optional else body (block or single statement)
    let mut else_body: Option<(&str, bool)> = None;
    let after_then = after_then.trim_start();
    if after_then.starts_with("else") {
        let after_else = after_then[4..].trim_start();
        if after_else.starts_with("if ") || after_else.starts_with("if(") {
            // Preserve full else-if chain
            else_body = Some((after_else, false));
        } else if after_else.starts_with('{') {
            let (block, _) = extract_braces(after_else)?;
            else_body = Some((block, true));
        } else if let Some((stmt, _)) = extract_statement(after_else) {
            else_body = Some((stmt, false));
        }
    }
    
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
        let result = if then_is_block {
            eval_block(ctx, then_body)?
        } else {
            eval_function_body(ctx, then_body)?
        };
        // Propagate return control
        if *ctx.get_loop_control() == crate::context::LoopControl::Return {
            return Some(ctx.get_return_value());
        }
        Some(result)
    } else if let Some((else_src, else_is_block)) = else_body {
        let result = if else_is_block {
            eval_block(ctx, else_src)?
        } else {
            eval_function_body(ctx, else_src)?
        };
        // Propagate return control
        if *ctx.get_loop_control() == crate::context::LoopControl::Return {
            return Some(ctx.get_return_value());
        }
        Some(result)
    } else {
        Some(Value::UNDEFINED)
    }
}

/// Parse while loop: "while (condition) { block }"
/// Optional label parameter for labeled statements
pub fn parse_while_loop(ctx: &mut JSContextImpl, src: &str, label: Option<&str>) -> Option<JSValue> {
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
    
    // Parse body (block or single statement)
    let (body, body_is_block) = if after_cond.starts_with('{') {
        let (block, _) = extract_braces(after_cond)?;
        (block, true)
    } else {
        let (stmt, _) = extract_statement(after_cond)?;
        (stmt, false)
    };
    
    // Pre-parse the loop body once to avoid re-parsing on every iteration
    let cached_stmts = parse_body_to_stmts(body);
    // Skip block scope creation when body has no let/const declarations
    let needs_block_scope = body_is_block && cached_stmts.as_ref().map_or(true, |stmts| {
        stmts.iter().any(|s| s.starts_with("let ") || s.starts_with("const "))
    });

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
        last = if let Some(ref stmts) = cached_stmts {
            if needs_block_scope {
                eval_block_cached(ctx, stmts)?
            } else {
                eval_function_body_cached(ctx, stmts)?
            }
        } else if body_is_block {
            eval_block(ctx, body)?
        } else {
            eval_function_body(ctx, body)?
        };
        
        // Check for loop control
        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::BreakLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    break;
                }
                // Propagate labeled break up
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                continue;
            }
            crate::context::LoopControl::ContinueLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    continue;
                }
                // Propagate labeled continue up
                break;
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
/// Optional label parameter for labeled statements
pub fn parse_for_of_loop(ctx: &mut JSContextImpl, header: &str, of_pos: usize, after_header: &str, label: Option<&str>) -> Option<JSValue> {
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

    let (body, body_is_block) = if after_header.starts_with('{') {
        let (block, _) = extract_braces(after_header)?;
        (block, true)
    } else {
        let (stmt, _) = extract_statement(after_header)?;
        (stmt, false)
    };

    // Pre-parse the loop body once to avoid re-parsing on every iteration
    let cached_stmts = parse_body_to_stmts(body);
    // Skip block scope creation when body has no let/const declarations
    let needs_block_scope = body_is_block && cached_stmts.as_ref().map_or(true, |stmts| {
        stmts.iter().any(|s| s.starts_with("let ") || s.starts_with("const "))
    });

    let loop_parent = ctx.current_env();
    let loop_env = js_new_object(ctx);
    js_set_property_str(ctx, loop_env, "__parent__", loop_parent);
    js_set_property_str(ctx, loop_env, "__block__", Value::TRUE);
    ctx.push_env(loop_env);

    let iter_val = eval_expr(ctx, iter_expr)?;

    if let Some(class_id) = ctx.object_class_id(iter_val) {
        if class_id == JSObjectClassEnum::Array as u32 {
            if let Some(len_val) = ctx.get_property_str(iter_val, b"length") {
                if let Some(len) = len_val.int32() {
                    let mut last = Value::UNDEFINED;
                    let env = if var_part.starts_with("var ") {
                        ctx.current_var_env()
                    } else {
                        ctx.current_env()
                    };

                    for i in 0..len {
                        let elem = js_get_property_uint32(ctx, iter_val, i as u32);
                        js_set_property_str(ctx, env, var_name, elem);
                        last = if let Some(ref stmts) = cached_stmts {
                            if needs_block_scope {
                                eval_block_cached(ctx, stmts)?
                            } else {
                                eval_function_body_cached(ctx, stmts)?
                            }
                        } else if body_is_block {
                            eval_block(ctx, body)?
                        } else {
                            eval_function_body(ctx, body)?
                        };

                        match ctx.get_loop_control() {
                            crate::context::LoopControl::Break => {
                                ctx.set_loop_control(crate::context::LoopControl::None);
                                break;
                            }
                            crate::context::LoopControl::BreakLabel(ref l) => {
                                if label == Some(l.as_str()) {
                                    ctx.set_loop_control(crate::context::LoopControl::None);
                                }
                                break;
                            }
                            crate::context::LoopControl::Continue => {
                                ctx.set_loop_control(crate::context::LoopControl::None);
                                continue;
                            }
                            crate::context::LoopControl::ContinueLabel(ref l) => {
                                if label == Some(l.as_str()) {
                                    ctx.set_loop_control(crate::context::LoopControl::None);
                                    continue;
                                }
                                break;
                            }
                            crate::context::LoopControl::Return => {
                                ctx.pop_env();
                                return Some(last);
                            }
                            crate::context::LoopControl::None => {}
                        }
                    }

                    ctx.pop_env();
                    return Some(last);
                }
            }
        }
    }

    ctx.pop_env();
    Some(Value::UNDEFINED)
}

/// Parse for...in loop
/// Optional label parameter for labeled statements
pub fn parse_for_in_loop(ctx: &mut JSContextImpl, header: &str, in_pos: usize, after_header: &str, label: Option<&str>) -> Option<JSValue> {
    let var_part = header[..in_pos].trim();
    let obj_expr = header[in_pos + 4..].trim();

    let (decl_kind, lvalue_src) = if var_part.starts_with("var ") {
        (Some("var"), var_part[4..].trim())
    } else if var_part.starts_with("let ") {
        (Some("let"), var_part[4..].trim())
    } else if var_part.starts_with("const ") {
        (Some("const"), var_part[6..].trim())
    } else {
        (None, var_part)
    };

    if !after_header.starts_with('{') {
        return None;
    }
    let (body, _) = extract_braces(after_header)?;

    // Pre-parse the loop body once to avoid re-parsing on every iteration
    let cached_stmts = parse_body_to_stmts(body);
    // for-in body is always a block; skip scope if no let/const
    let needs_block_scope_forin = cached_stmts.as_ref().map_or(true, |stmts| {
        stmts.iter().any(|s| s.starts_with("let ") || s.starts_with("const "))
    });

    let loop_parent = ctx.current_env();
    let loop_env = js_new_object(ctx);
    js_set_property_str(ctx, loop_env, "__parent__", loop_parent);
    js_set_property_str(ctx, loop_env, "__block__", Value::TRUE);
    ctx.push_env(loop_env);

    let obj_val = eval_expr(ctx, obj_expr)?;
    let keys = get_object_keys(ctx, obj_val)?;

    let mut last = Value::UNDEFINED;
    let env = if decl_kind == Some("var") {
        ctx.current_var_env()
    } else {
        ctx.current_env()
    };

    if decl_kind.is_some() && !is_identifier(lvalue_src) {
        return None;
    }

    for key in keys {
        let key_val = js_new_string(ctx, &key);
        if let Some(_) = decl_kind {
            js_set_property_str(ctx, env, lvalue_src, key_val);
        } else {
            let (base, key) = parse_lvalue(ctx, lvalue_src)?;
            match key {
                LValueKey::Index(idx) => {
                    js_set_property_uint32(ctx, base, idx, key_val);
                }
                LValueKey::Name(name) => {
                    js_set_property_str(ctx, base, &name, key_val);
                }
            }
        }

        last = if let Some(ref stmts) = cached_stmts {
            if needs_block_scope_forin {
                eval_block_cached(ctx, stmts)?
            } else {
                eval_function_body_cached(ctx, stmts)?
            }
        } else {
            eval_block(ctx, body)?
        };

        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::BreakLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                }
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
            crate::context::LoopControl::ContinueLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                } else {
                    break;
                }
            }
            crate::context::LoopControl::Return => {
                break;
            }
            crate::context::LoopControl::None => {}
        }
    }

    ctx.pop_env();
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
/// Optional label parameter for labeled statements
pub fn parse_for_loop(ctx: &mut JSContextImpl, src: &str, label: Option<&str>) -> Option<JSValue> {
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
        return parse_for_in_loop(ctx, header, in_pos, after_header, label);
    }

    if let Some(of_pos) = find_for_of_keyword(header) {
        return parse_for_of_loop(ctx, header, of_pos, after_header, label);
    }

    let parts: Vec<&str> = header.split(';').collect();
    if parts.len() != 3 {
        return None;
    }
    let init = parts[0].trim();
    let condition = parts[1].trim();
    let update = parts[2].trim();
    
    // Extract body - can be either { block } or single statement
    let (body, body_is_block) = if after_header.starts_with('{') {
        let (b, _) = extract_braces(after_header)?;
        (b.to_string(), true)
    } else {
        // Single statement body (e.g., "for(...) statement;")
        (after_header.to_string(), false)
    };
    
    // Pre-parse the loop body once to avoid re-parsing on every iteration
    let cached_stmts = parse_body_to_stmts(&body);
    // Skip block scope creation when body has no let/const declarations
    let needs_block_scope = body_is_block && cached_stmts.as_ref().map_or(true, |stmts| {
        stmts.iter().any(|s| s.starts_with("let ") || s.starts_with("const "))
    });

    let loop_parent = ctx.current_env();
    let loop_env = js_new_object(ctx);
    js_set_property_str(ctx, loop_env, "__parent__", loop_parent);
    js_set_property_str(ctx, loop_env, "__block__", Value::TRUE);
    ctx.push_env(loop_env);

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
        
        last = if let Some(ref stmts) = cached_stmts {
            if needs_block_scope {
                eval_block_cached(ctx, stmts)?
            } else {
                eval_function_body_cached(ctx, stmts)?
            }
        } else if body_is_block {
            eval_block(ctx, &body)?
        } else {
            eval_function_body(ctx, &body)?
        };
        
        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::BreakLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    break;
                }
                // Propagate labeled break up
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
            crate::context::LoopControl::ContinueLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    // Continue this loop
                } else {
                    // Propagate labeled continue up
                    break;
                }
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
    
    ctx.pop_env();
    Some(last)
}

/// Parse do...while loop
/// Optional label parameter for labeled statements
pub fn parse_do_while_loop(ctx: &mut JSContextImpl, src: &str, label: Option<&str>) -> Option<JSValue> {
    let s = src.trim();
    let rest = if s.starts_with("do ") {
        &s[3..]
    } else if s.starts_with("do{") {
        &s[2..]
    } else {
        return None;
    };

    let rest = rest.trim_start();
    let (body_part, after_body) = split_do_while_body(rest)?;

    let (body, body_is_block) = if body_part.starts_with('{') {
        let (block, _) = extract_braces(body_part)?;
        (block, true)
    } else {
        (body_part, false)
    };

    if !after_body.starts_with("while") {
        return None;
    }
    let after_while = after_body[5..].trim_start();

    if !after_while.starts_with('(') {
        return None;
    }
    let (condition, _) = extract_paren(after_while)?;

    // Pre-parse the loop body once to avoid re-parsing on every iteration
    let cached_stmts = parse_body_to_stmts(body);
    // Skip block scope creation when body has no let/const declarations
    let needs_block_scope = body_is_block && cached_stmts.as_ref().map_or(true, |stmts| {
        stmts.iter().any(|s| s.starts_with("let ") || s.starts_with("const "))
    });

    let mut last = if let Some(ref stmts) = cached_stmts {
        if needs_block_scope {
            eval_block_cached(ctx, stmts)?
        } else {
            eval_function_body_cached(ctx, stmts)?
        }
    } else if body_is_block {
        eval_block(ctx, body)?
    } else {
        eval_function_body(ctx, body)?
    };
    loop {
        match ctx.get_loop_control() {
            crate::context::LoopControl::Break => {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            crate::context::LoopControl::BreakLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    break;
                }
                // Propagate labeled break up
                break;
            }
            crate::context::LoopControl::Continue => {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
            crate::context::LoopControl::ContinueLabel(ref l) => {
                if label == Some(l.as_str()) {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    // Continue this loop
                } else {
                    // Propagate labeled continue up
                    break;
                }
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
        last = if let Some(ref stmts) = cached_stmts {
            if needs_block_scope {
                eval_block_cached(ctx, stmts)?
            } else {
                eval_function_body_cached(ctx, stmts)?
            }
        } else if body_is_block {
            eval_block(ctx, body)?
        } else {
            eval_function_body(ctx, body)?
        };
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

    let try_result = eval_block(ctx, try_body);

    let mut result = match try_result {
        Some(val) if val.is_exception() => {
            if let Some(body) = catch_body {
                let exception_val = ctx.get_exception();
                ctx.set_exception(Value::UNDEFINED);

                let parent_env = ctx.current_env();
                let catch_env = js_new_object(ctx);
                js_set_property_str(ctx, catch_env, "__parent__", parent_env);
                ctx.push_env(catch_env);
                if let Some(param) = catch_param {
                    js_set_property_str(ctx, catch_env, param, exception_val);
                }

                let out = match eval_block(ctx, body) {
                    Some(v) => v,
                    None => return None,
                };
                ctx.pop_env();
                out
            } else {
                val
            }
        }
        Some(val) => val,
        None => {
            if let Some(body) = catch_body {
                ctx.set_exception(Value::UNDEFINED);

                let parent_env = ctx.current_env();
                let catch_env = js_new_object(ctx);
                js_set_property_str(ctx, catch_env, "__parent__", parent_env);
                ctx.push_env(catch_env);
                if let Some(param) = catch_param {
                    js_set_property_str(ctx, catch_env, param, Value::UNDEFINED);
                }

                let out = match eval_block(ctx, body) {
                    Some(v) => v,
                    None => return None,
                };
                ctx.pop_env();
                out
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

        match eval_block(ctx, body) {
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

    let parent = ctx.current_env();
    let env = js_new_object(ctx);
    js_set_property_str(ctx, env, "__parent__", parent);
    js_set_property_str(ctx, env, "__block__", Value::TRUE);
    ctx.push_env(env);

    let mut matched = false;
    let mut last = Value::UNDEFINED;
    let mut found_default: Option<&str> = None;

    for (case_expr, case_body) in &cases {
        if case_expr.is_none() {
            found_default = Some(case_body);
            if matched {
                last = eval_function_body(ctx, case_body)?;
                if *ctx.get_loop_control() == crate::context::LoopControl::Break {
                    ctx.set_loop_control(crate::context::LoopControl::None);
                    break;
                }
            }
            continue;
        }

        if matched {
            last = eval_function_body(ctx, case_body)?;
            if *ctx.get_loop_control() == crate::context::LoopControl::Break {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
            continue;
        }

        let case_val = eval_expr(ctx, case_expr.unwrap())?;

        if switch_val.0 == case_val.0 {
            matched = true;
            last = eval_function_body(ctx, case_body)?;
            if *ctx.get_loop_control() == crate::context::LoopControl::Break {
                ctx.set_loop_control(crate::context::LoopControl::None);
                break;
            }
        }
    }

    if !matched {
        if let Some(default_body) = found_default {
            last = eval_function_body(ctx, default_body)?;
            if *ctx.get_loop_control() == crate::context::LoopControl::Break {
                ctx.set_loop_control(crate::context::LoopControl::None);
            }
        }
    }
    ctx.pop_env();
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
        if tail.trim().is_empty() {
            let env = ctx
                .resolve_binding_env(base_str)
                .unwrap_or_else(|| ctx.current_env());
            return Some((env, LValueKey::Name(base_str.to_string())));
        }
        ctx.resolve_binding_value(base_str)
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
                if next == b'>' || prev == b'=' || next == b'=' || prev == b'!' || prev == b'<' || prev == b'>' {
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

/// Split expression on top-level `instanceof`.
pub fn split_instanceof(src: &str) -> Option<(&str, &str)> {
    let s = src.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    let token = b"instanceof";
    let is_ident_char = |b: u8| -> bool {
        (b'A'..=b'Z').contains(&b)
            || (b'a'..=b'z').contains(&b)
            || (b'0'..=b'9').contains(&b)
            || b == b'_'
    };
    let mut i = 0usize;
    while i + token.len() <= bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == string_delim {
                in_string = false;
            } else if b == b'\\' {
                i += 1; // skip escaped
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
            b'[' | b'{' | b'(' => {
                depth += 1;
                i += 1;
                continue;
            }
            b']' | b'}' | b')' => {
                depth -= 1;
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth == 0 && bytes[i..].starts_with(token) {
            let before = if i == 0 { None } else { Some(bytes[i - 1]) };
            let after = bytes.get(i + token.len()).copied();
            if before.map(is_ident_char).unwrap_or(false) || after.map(is_ident_char).unwrap_or(false) {
                i += 1;
                continue;
            }
            let lhs = s[..i].trim();
            let rhs = s[i + token.len()..].trim();
            if !lhs.is_empty() && !rhs.is_empty() {
                return Some((lhs, rhs));
            }
            return None;
        }
        i += 1;
    }
    None
}

/// Split expression on top-level ` in ` (property existence check).
pub fn split_in_operator(src: &str) -> Option<(&str, &str)> {
    let s = src.trim();
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_delim = 0u8;
    // Match " in " with surrounding spaces to distinguish from "for...in" and identifiers like "index"
    let token = b" in ";
    let mut i = 0usize;
    while i + token.len() <= bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == string_delim {
                in_string = false;
            } else if b == b'\\' {
                i += 1; // skip escaped
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
            b'[' | b'{' | b'(' => {
                depth += 1;
                i += 1;
                continue;
            }
            b']' | b'}' | b')' => {
                depth -= 1;
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth == 0 && bytes[i..].starts_with(token) {
            let lhs = s[..i].trim();
            let rhs = s[i + token.len()..].trim();
            if !lhs.is_empty() && !rhs.is_empty() {
                return Some((lhs, rhs));
            }
            return None;
        }
        i += 1;
    }
    None
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

    let env = ctx.current_env();
    js_set_property_str(ctx, func, "__env__", env);

    // Create prototype property with constructor pointing back to the function
    let prototype = js_new_object(ctx);
    js_set_property_str(ctx, prototype, "constructor", func);
    js_set_property_str(ctx, func, "prototype", prototype);

    Some(func)
}

/// Call a closure with arguments
pub fn call_closure(ctx: &mut JSContextImpl, func: JSValue, args: &[JSValue]) -> Option<JSValue> {
    call_closure_with_this(ctx, func, Value::UNDEFINED, args)
}

/// Call a closure with a specific `this` binding and arguments
pub fn call_closure_with_this(ctx: &mut JSContextImpl, func: JSValue, this_val: JSValue, args: &[JSValue]) -> Option<JSValue> {
    let params_val = js_get_property_str(ctx, func, "__params__");
    let body_val = js_get_property_str(ctx, func, "__body__");

    #[derive(Clone)]
    struct ParamSpec {
        name: String,
        default: Option<String>,
        rest: bool,
    }
    let mut param_specs: Vec<ParamSpec> = Vec::new();
    {
        let params_bytes = ctx.string_bytes(params_val)?;
        let params_str = core::str::from_utf8(params_bytes).ok()?;
        if !params_str.is_empty() {
            for raw in params_str.split(',') {
                let raw = raw.trim();
                if raw.is_empty() {
                    continue;
                }
                if raw.starts_with("...") {
                    let name = raw[3..].trim().to_string();
                    param_specs.push(ParamSpec { name, default: None, rest: true });
                    break;
                }
                if let Some(eq_pos) = raw.find('=') {
                    let name = raw[..eq_pos].trim().to_string();
                    let default = raw[eq_pos + 1..].trim().to_string();
                    param_specs.push(ParamSpec { name, default: Some(default), rest: false });
                } else {
                    param_specs.push(ParamSpec { name: raw.to_string(), default: None, rest: false });
                }
            }
        }
    }

    // --- Body cache: parse once, reuse on subsequent calls ---
    let body_key = body_val.0 as u64;
    let use_cached = ctx.get_body_cache(body_key).is_some();

    if !use_cached {
        // First call: parse body and store in cache
        let body_bytes = ctx.string_bytes(body_val)?;
        let body_str = core::str::from_utf8(body_bytes).ok()?.to_string();
        let cleaned = match crate::evals::strip_comments_checked(&body_str) {
            Ok(s) => crate::evals::normalize_line_continuations(&s),
            Err(_) => body_str.clone(),
        };
        let stmts = crate::evals::split_statements(&cleaned)
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<String>>();
        ctx.set_body_cache(body_key, stmts);
    }

    // Retrieve cached statements (borrow from cache, clone to own them for execution)
    let cached_stmts = ctx.get_body_cache(body_key)?.clone();

    // Check if function has a name (for recursive named function expressions)
    let func_name_val = js_get_property_str(ctx, func, "name");
    let func_name: Option<String> = if !func_name_val.is_undefined() {
        ctx.string_bytes(func_name_val)
            .and_then(|b| core::str::from_utf8(b).ok())
            .map(|s| s.to_string())
    } else {
        None
    };

    let parent_env = js_get_property_str(ctx, func, "__env__");
    let env = js_new_object(ctx);
    js_set_property_str(ctx, env, "__parent__", parent_env);
    js_set_property_str(ctx, env, "__var_env__", Value::TRUE);

    // Bind `this` in the function scope
    js_set_property_str(ctx, env, "this", this_val);

    ctx.push_env(env);
    predeclare_block_bindings_from_stmts(ctx, &cached_stmts);

    // Bind function name in local scope for recursive calls
    if let Some(name) = func_name {
        js_set_property_str(ctx, env, &name, func);
    }

    // Populate `arguments` for function scope.
    let args_obj = js_new_array(ctx, args.len() as i32);
    if !args_obj.is_exception() {
        for (i, val) in args.iter().enumerate() {
            let _ = js_set_property_uint32(ctx, args_obj, i as u32, *val);
        }
        js_set_property_str(ctx, env, "arguments", args_obj);
    }

    let mut arg_index = 0usize;
    for spec in &param_specs {
        if spec.rest {
            let remaining = &args[arg_index..];
            let arr = js_new_array(ctx, remaining.len() as i32);
            for (i, val) in remaining.iter().enumerate() {
                let _ = js_set_property_uint32(ctx, arr, i as u32, *val);
            }
            js_set_property_str(ctx, env, &spec.name, arr);
            break;
        }
        let mut arg_val = args.get(arg_index).copied().unwrap_or(Value::UNDEFINED);
        if arg_val == Value::UNDEFINED {
            if let Some(expr) = &spec.default {
                arg_val = eval_expr(ctx, expr).unwrap_or(Value::UNDEFINED);
            }
        }
        js_set_property_str(ctx, env, &spec.name, arg_val);
        arg_index += 1;
    }

    let result = eval_function_body_cached(ctx, &cached_stmts);

    if *ctx.get_loop_control() == crate::context::LoopControl::Return {
        ctx.set_loop_control(crate::context::LoopControl::None);
    }

    ctx.pop_env();

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
