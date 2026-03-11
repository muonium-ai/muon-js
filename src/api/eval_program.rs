#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use super::number_fmt::*;
#[allow(unused_imports)]
use super::typed_array::*;
use crate::types::*;
use crate::value::Value;
use crate::helpers::{number_to_value, is_identifier, flatten_array, contains_arith_op};
use crate::json::parse_json;
use crate::evals::{
    eval_value,
    split_top_level,
    has_top_level_comma,
    split_statements,
    normalize_line_continuations,
    is_truthy,
};
use crate::parser::*;
use fancy_regex::Regex;

pub fn parse_body_to_stmts(body: &str) -> Option<Vec<String>> {
    let stripped = match crate::evals::strip_comments_checked(body) {
        Ok(s) => s,
        Err(_) => return None,
    };
    let cleaned = normalize_line_continuations(&stripped);
    let stmts = split_statements(&cleaned)?;
    Some(
        stmts
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

pub fn eval_function_body(ctx: &mut JSContextImpl, body: &str) -> Option<JSValue> {
    let stripped = match crate::evals::strip_comments_checked(body) {
        Ok(s) => s,
        Err(pos) => {
            ctx.set_error_offset(pos);
            return None;
        }
    };
    let cleaned = normalize_line_continuations(&stripped);
    let stmts = split_statements(&cleaned)?;
    let mut last = Value::UNDEFINED;
    let mut search_idx = 0usize;
    
    for stmt in stmts {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(pos) = cleaned[search_idx..].find(trimmed) {
            let base = search_idx + pos;
            ctx.set_current_stmt_offset(base);
            search_idx = base + trimmed.len();
        }
        ctx.clear_error_offset();
        
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
        match eval_expr(ctx, trimmed) {
            Some(val) => {
                last = val;
            }
            None => {
                let pos = find_syntax_error_offset(trimmed);
                ctx.set_error_offset(ctx.current_stmt_offset() + pos);
                return None;
            }
        }
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

/// Execute a function body from pre-parsed cached statements.
/// Skips strip_comments, normalize_line_continuations, and split_statements.
pub fn eval_function_body_cached(ctx: &mut JSContextImpl, stmts: &[String]) -> Option<JSValue> {
    let mut last = Value::UNDEFINED;

    for stmt in stmts {
        let trimmed = stmt.as_str();
        if trimmed.is_empty() {
            continue;
        }
        ctx.set_current_stmt_offset(0);
        ctx.clear_error_offset();

        // Fast dispatch: use first byte to skip impossible keyword checks.
        // For first bytes that can't start any keyword, go directly to eval_expr.
        // For 'r' (could be "return" but also "redis.call(...)"), check "return" and skip to eval_expr if not.
        let first = trimmed.as_bytes()[0];
        let is_keyword_possible = match first {
            b'b' | b'c' | b't' | b'f' | b'i' | b'w' | b'd' | b's' | b'{' | b'v' | b'l' => true,
            b'r' => {
                // Only "return" starts with 'r'; if it's not "return", go to eval_expr
                if trimmed == "return" || trimmed.starts_with("return ") {
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if !is_keyword_possible {
            // Labeled statements can start with any identifier, so they can
            // hit this fast path (e.g. "x: { break x; }"). Handle them before
            // falling back to expression parsing.
            if trimmed.as_bytes().contains(&b':') {
                if let Some(label_result) = parse_labeled_statement(ctx, trimmed) {
                    last = label_result?;
                    if *ctx.get_loop_control() != crate::context::LoopControl::None {
                        return Some(last);
                    }
                    continue;
                }
            }
            // Simple expression statement — go directly to eval_expr
            match eval_expr(ctx, trimmed) {
                Some(val) => {
                    last = val;
                    if last.is_exception() {
                        return Some(last);
                    }
                    if *ctx.get_loop_control() != crate::context::LoopControl::None {
                        return Some(last);
                    }
                    continue;
                }
                None => {
                    return None;
                }
            }
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
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }

        // Check for while loop
        if trimmed.starts_with("while ") || trimmed.starts_with("while(") {
            last = parse_while_loop(ctx, trimmed, None)?;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }

        // Check for for loop
        if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
            last = parse_for_loop(ctx, trimmed, None)?;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }

        // Check for do...while loop
        if trimmed.starts_with("do ") || trimmed.starts_with("do{") {
            last = parse_do_while_loop(ctx, trimmed, None)?;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }

        // Check for switch statement
        if trimmed.starts_with("switch ") || trimmed.starts_with("switch(") {
            last = parse_switch_statement(ctx, trimmed)?;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }

        // Check for bare block statement: { ... }
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            let (block_content, _) = extract_braces(trimmed)?;
            last = eval_block(ctx, block_content)?;
            if *ctx.get_loop_control() != crate::context::LoopControl::None {
                return Some(last);
            }
            continue;
        }

        // Execute statement
        match eval_expr(ctx, trimmed) {
            Some(val) => {
                last = val;
            }
            None => {
                let pos = find_syntax_error_offset(trimmed);
                ctx.set_error_offset(ctx.current_stmt_offset() + pos);
                return None;
            }
        }
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

/// Evaluate a script body that supports top-level `return`, like a function body
/// but without the overhead of creating/calling a function.
/// Used by JS_EVAL_SCRIPT flag to eliminate function wrapping overhead.
/// Simple FNV-1a hash for script caching.
#[inline]
pub(crate) fn fnv1a_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Count unescaped occurrences of a quote character in a string.
#[inline]
pub(crate) fn count_unescaped_quotes(s: &str, q: u8) -> usize {
    let bytes = s.as_bytes();
    let mut count = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2; // skip escaped char
            continue;
        }
        if bytes[i] == q {
            count += 1;
        }
        i += 1;
    }
    count
}

pub(crate) fn eval_script_body(ctx: &mut JSContextImpl, src: &str) -> JSValue {
    // Fast path: single-line return statement (very common for Redis scripts)
    let trimmed_src = src.trim();
    if trimmed_src.starts_with("return ") && !trimmed_src.contains('\n') {
        let expr = trimmed_src[7..].trim().trim_end_matches(';');
        if !expr.is_empty() {
            if let Some(val) = eval_expr(ctx, expr) {
                return val;
            }
        }
    }

    // Check script cache first (avoids re-parsing on repeated EVAL calls)
    let script_hash = fnv1a_hash(src.as_bytes());
    let stmts: Vec<String> = if let Some(cached) = ctx.get_script_cache(script_hash) {
        cached.to_vec()
    } else {
        // Parse: strip comments, normalize, split statements
        let stripped = if !src.contains("//") && !src.contains("/*") {
            src.to_string()
        } else {
            match crate::evals::strip_comments_checked(src) {
                Ok(s) => s.into_owned(),
                Err(pos) => {
                    ctx.set_error_offset(pos);
                    return js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "syntax error");
                }
            }
        };
        let cleaned = if !stripped.contains("\\\n") && !stripped.contains("\\\r") {
            stripped
        } else {
            normalize_line_continuations(&stripped).into_owned()
        };
        let parsed = match split_statements(&cleaned) {
            Some(s) => s,
            None => return js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "syntax error"),
        };
        // Cache the parsed statements (trimmed, non-empty)
        let trimmed_stmts: Vec<String> = parsed.iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        ctx.set_script_cache(script_hash, trimmed_stmts.clone());
        trimmed_stmts
    };

    // --- Bytecode fast path: compile whole script to bytecode ---
    // Try to compile the script statements to bytecode and run via VM.
    // This avoids re-parsing condition/update/body text every loop iteration.
    let bc_cached = ctx.get_bytecode_cache(script_hash).cloned();
    match bc_cached {
        Some(Some(ref module)) => {
            // Run cached bytecode
            let mut vm = crate::vm::VM::new();
            return vm.run_module(ctx, module);
        }
        Some(None) => {
            // Previously tried and failed to compile — fall through
        }
        None => {
            let sc = crate::compiler::StmtCompiler::new(ctx);
            #[allow(unused_mut)]
            match sc.compile_stmts(&stmts) {
                Ok(module) => {
                    let mut vm = crate::vm::VM::new();
                    let result = vm.run_module(ctx, &module);
                    ctx.set_bytecode_cache(script_hash, Some(module));
                    return result;
                }
                Err(_) => {
                    ctx.set_bytecode_cache(script_hash, None);
                }
            }
        }
    }

    let mut last = Value::UNDEFINED;
    for stmt in &stmts {
        let trimmed = stmt.as_str();
        ctx.clear_error_offset();
        // Handle return statement at top level
        if trimmed.starts_with("return ") || trimmed.starts_with("return;") {
            let expr = if trimmed.starts_with("return;") {
                ""
            } else {
                trimmed[7..].trim()
            };
            if expr.is_empty() {
                return Value::UNDEFINED;
            }
            // Fast path for return of simple literals (string, number, bool, null)
            let ebytes = expr.as_bytes();
            if ebytes.len() >= 2 {
                let first = ebytes[0];
                let last_byte = ebytes[ebytes.len() - 1];
                // String literal: "..." or '...'
                if (first == b'"' && last_byte == b'"') || (first == b'\'' && last_byte == b'\'') {
                    if let Some(val) = eval_value(ctx, expr) {
                        return val;
                    }
                }
                // Integer literal
                if first.is_ascii_digit() && ebytes.iter().all(|b| b.is_ascii_digit()) {
                    if let Ok(n) = expr.parse::<i32>() {
                        return js_new_int32(ctx, n);
                    }
                }
            }
            return match eval_expr(ctx, expr) {
                Some(val) => val,
                None => {
                    let off = find_syntax_error_offset(trimmed);
                    ctx.set_error_offset(ctx.current_stmt_offset() + off);
                    js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "syntax error")
                }
            };
        }
        if trimmed == "return" {
            return Value::UNDEFINED;
        }
        // Delegate to eval_program for all other statement types
        match eval_program_stmt(ctx, trimmed) {
            StmtResult::Value(val) => {
                last = val;
                if last.is_exception() {
                    return last;
                }
            }
            StmtResult::Error => {
                return js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "syntax error");
            }
            StmtResult::LoopControl => return last,
        }
    }
    last
}

/// Result of evaluating a single statement in eval_program
enum StmtResult {
    Value(JSValue),
    Error,
    LoopControl,
}

/// Evaluate a single statement (factored out from eval_program for reuse).
fn eval_program_stmt(ctx: &mut JSContextImpl, trimmed: &str) -> StmtResult {
    // Fast dispatch: check first byte to skip keyword prefix chain for expression statements.
    // Keywords that start statement types: b(reak), c(ontinue/const), d(o), f(or/function),
    // i(f), l(et), s(witch), t(hrow/try), v(ar), w(hile), and '{' for bare blocks.
    // Any other first byte must be an expression statement.
    let first = match trimmed.as_bytes().first() {
        Some(&b) => b,
        None => return StmtResult::Value(Value::UNDEFINED),
    };
    match first {
        b'b' => {
            if trimmed == "break" {
                ctx.set_loop_control(crate::context::LoopControl::Break);
                return StmtResult::LoopControl;
            }
            // fall through to expression
        }
        b'c' => {
            if trimmed == "continue" {
                ctx.set_loop_control(crate::context::LoopControl::Continue);
                return StmtResult::LoopControl;
            }
            // 'const' is handled by eval_expr
            // fall through to expression
        }
        b'f' => {
            if trimmed.starts_with("function ") {
                if let Some(val) = parse_function_declaration(ctx, trimmed) {
                    return StmtResult::Value(val);
                }
                if let Some(pos) = find_function_error_pos(trimmed) {
                    ctx.set_error_offset(ctx.current_stmt_offset() + pos);
                }
                return StmtResult::Error;
            }
            if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
                if let Some(val) = parse_for_loop(ctx, trimmed, None) {
                    return StmtResult::Value(val);
                }
                ctx.set_error_offset(ctx.current_stmt_offset());
                return StmtResult::Error;
            }
            // fall through to expression (e.g., function call starting with 'f')
        }
        b'i' => {
            if trimmed.starts_with("if ") || trimmed.starts_with("if(") {
                if let Some(val) = parse_if_statement(ctx, trimmed) {
                    return StmtResult::Value(val);
                }
                ctx.set_error_offset(ctx.current_stmt_offset());
                return StmtResult::Error;
            }
            // fall through to expression
        }
        b'w' => {
            if trimmed.starts_with("while ") || trimmed.starts_with("while(") {
                if let Some(val) = parse_while_loop(ctx, trimmed, None) {
                    return StmtResult::Value(val);
                }
                ctx.set_error_offset(ctx.current_stmt_offset());
                return StmtResult::Error;
            }
            // fall through to expression
        }
        b'd' => {
            if trimmed.starts_with("do ") || trimmed.starts_with("do{") {
                if let Some(val) = parse_do_while_loop(ctx, trimmed, None) {
                    return StmtResult::Value(val);
                }
                ctx.set_error_offset(ctx.current_stmt_offset());
                return StmtResult::Error;
            }
            // fall through to expression
        }
        b's' => {
            if trimmed.starts_with("switch ") || trimmed.starts_with("switch(") {
                if let Some(val) = parse_switch_statement(ctx, trimmed) {
                    return StmtResult::Value(val);
                }
                ctx.set_error_offset(ctx.current_stmt_offset());
                return StmtResult::Error;
            }
            // fall through to expression
        }
        b't' => {
            if trimmed.starts_with("throw ") || trimmed == "throw" {
                if trimmed == "throw" {
                    ctx.set_exception(Value::UNDEFINED);
                    return StmtResult::Value(Value::EXCEPTION);
                }
                let expr = trimmed[6..].trim();
                if let Some(val) = eval_expr(ctx, expr) {
                    ctx.set_exception(val);
                    return StmtResult::Value(Value::EXCEPTION);
                }
                ctx.set_exception(Value::UNDEFINED);
                return StmtResult::Value(Value::EXCEPTION);
            }
            if trimmed.starts_with("try ") || trimmed.starts_with("try{") {
                if let Some(val) = parse_try_catch(ctx, trimmed) {
                    return StmtResult::Value(val);
                }
                return StmtResult::Error;
            }
            // fall through to expression
        }
        b'{' => {
            if trimmed.ends_with('}') {
                if let Some((block_content, _)) = extract_braces(trimmed) {
                    if let Some(val) = eval_block(ctx, block_content) {
                        if *ctx.get_loop_control() != crate::context::LoopControl::None {
                            return StmtResult::LoopControl;
                        }
                        return StmtResult::Value(val);
                    }
                }
                return StmtResult::Error;
            }
            // fall through to expression
        }
        _ => {
            // First byte doesn't match any keyword — skip straight to eval_expr.
            // This is the fast path for expression statements like `redis.call(...)`,
            // variable assignments starting with non-keyword identifiers, etc.
        }
    }

    // Labeled statement check (only for identifiers that could be labels)
    if first.is_ascii_alphabetic() || first == b'_' || first == b'$' {
        if let Some(label_result) = parse_labeled_statement(ctx, trimmed) {
            match label_result {
                Some(val) => {
                    if *ctx.get_loop_control() != crate::context::LoopControl::None {
                        return StmtResult::LoopControl;
                    }
                    return StmtResult::Value(val);
                }
                None => return StmtResult::Error,
            }
        }
    }

    // Expression (most common path for non-keyword statements)
    match eval_expr(ctx, trimmed) {
        Some(val) => StmtResult::Value(val),
        None => {
            let pos = find_syntax_error_offset(trimmed);
            ctx.set_error_offset(ctx.current_stmt_offset() + pos);
            StmtResult::Error
        }
    }
}


pub fn eval_program(ctx: &mut JSContextImpl, src: &str) -> Option<JSValue> {
    let stripped = match crate::evals::strip_comments_checked(src) {
        Ok(s) => s,
        Err(pos) => {
            ctx.set_error_offset(pos);
            return None;
        }
    };
    let cleaned = normalize_line_continuations(&stripped);
    let stmts = split_statements(&cleaned)?;
    let mut last = Value::UNDEFINED;
    let mut any = false;
    let mut search_idx = 0usize;
    for stmt in stmts {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(pos) = cleaned[search_idx..].find(trimmed) {
            let base = search_idx + pos;
            ctx.set_current_stmt_offset(base);
            search_idx = base + trimmed.len();
        }
        ctx.clear_error_offset();
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
            if let Some(pos) = find_function_error_pos(trimmed) {
                ctx.set_error_offset(ctx.current_stmt_offset() + pos);
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
            ctx.set_error_offset(ctx.current_stmt_offset());
            return None;
        }
        // Check for while loop
        if trimmed.starts_with("while ") || trimmed.starts_with("while(") {
            if let Some(val) = parse_while_loop(ctx, trimmed, None) {
                last = val;
                any = true;
                continue;
            }
            ctx.set_error_offset(ctx.current_stmt_offset());
            return None;
        }
        // Check for for loop
        if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
            if let Some(val) = parse_for_loop(ctx, trimmed, None) {
                last = val;
                any = true;
                continue;
            }
            ctx.set_error_offset(ctx.current_stmt_offset());
            return None;
        }
        // Check for do...while loop
        if trimmed.starts_with("do ") || trimmed.starts_with("do{") {
            if let Some(val) = parse_do_while_loop(ctx, trimmed, None) {
                last = val;
                any = true;
                continue;
            }
            ctx.set_error_offset(ctx.current_stmt_offset());
            return None;
        }
        // Check for switch statement
        if trimmed.starts_with("switch ") || trimmed.starts_with("switch(") {
            if let Some(val) = parse_switch_statement(ctx, trimmed) {
                last = val;
                any = true;
                continue;
            }
            ctx.set_error_offset(ctx.current_stmt_offset());
            return None;
        }
        // Check for throw statement
        if trimmed.starts_with("throw ") {
            let expr = &trimmed[6..]; // skip "throw "
            if let Some(val) = eval_expr(ctx, expr.trim()) {
                ctx.set_exception(val);
                return Some(Value::EXCEPTION);
            }
            ctx.set_error_offset(ctx.current_stmt_offset());
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
        match eval_expr(ctx, trimmed) {
            Some(val) => {
                last = val;
            }
            None => {
                let pos = find_syntax_error_offset(trimmed);
                ctx.set_error_offset(ctx.current_stmt_offset() + pos);
                return None;
            }
        }
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

/// Direct C function dispatch — public for VM inline fast path.
#[inline]
pub fn call_c_function_direct(
    ctx: &mut JSContextImpl,
    func_idx: i32,
    params: JSValue,
    this_val: JSValue,
    args: &[JSValue],
) -> JSValue {
    call_c_function(ctx, func_idx, params, this_val, args)
}

pub(crate) fn call_c_function(
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




pub(crate) struct ExprParser<'a> {
    pub(crate) input: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> ExprParser<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    pub(crate) fn parse_expr(&mut self) -> Result<f64, ()> {
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

    pub(crate) fn parse_term(&mut self) -> Result<f64, ()> {
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

    pub(crate) fn parse_factor(&mut self) -> Result<f64, ()> {
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

    pub(crate) fn parse_number(&mut self) -> Result<f64, ()> {
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

    pub(crate) fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
}

pub(crate) struct ArithParser<'a> {
    pub(crate) ctx: *mut JSContextImpl,
    pub(crate) input: &'a [u8],
    pub(crate) pos: usize,
    pub(crate) base_offset: usize,
}

impl<'a> ArithParser<'a> {
    pub(crate) fn new(ctx: &mut JSContextImpl, input: &'a [u8], base_offset: usize) -> Self {
        Self {
            ctx,
            input,
            pos: 0,
            base_offset,
        }
    }

    pub(crate) fn set_error_at(&mut self, pos: usize) {
        let ctx = unsafe { &mut *self.ctx };
        ctx.set_error_offset(self.base_offset + pos);
    }

    pub(crate) fn parse_expr(&mut self) -> Result<JSValue, ()> {
        self.parse_logical_or()
    }

    pub(crate) fn parse_logical_or(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_logical_and(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_bitwise_or(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_bitwise_xor(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_bitwise_and(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_comparison(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_shift(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_additive()?;
        loop {
            self.skip_ws();
            if self.peek() == Some(b'<') && self.peek_at(1) == Some(b'<') {
                let op_pos = self.pos;
                self.pos += 2;
                let rhs = self.parse_additive()?;
                self.set_error_at(op_pos);
                value = self.left_shift(value, rhs)?;
            } else if self.peek() == Some(b'>') && self.peek_at(1) == Some(b'>') {
                let op_pos = self.pos;
                self.pos += 2;
                if self.peek() == Some(b'>') {
                    self.pos += 1;
                    let rhs = self.parse_additive()?;
                    self.set_error_at(op_pos);
                    value = self.unsigned_right_shift(value, rhs)?;
                } else {
                    let rhs = self.parse_additive()?;
                    self.set_error_at(op_pos);
                    value = self.right_shift(value, rhs)?;
                }
            } else {
                break;
            }
        }
        Ok(value)
    }

    pub(crate) fn parse_additive(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_term()?;
        loop {
            self.skip_ws();
            let op = match self.peek() {
                Some(b'+') => b'+',
                Some(b'-') => b'-',
                _ => break,
            };
            let op_pos = self.pos;
            self.pos += 1;
            let rhs = self.parse_term()?;
            self.set_error_at(op_pos);
            value = if op == b'+' {
                self.add_values(value, rhs)?
            } else {
                self.sub_values(value, rhs)?
            };
        }
        Ok(value)
    }

    pub(crate) fn parse_term(&mut self) -> Result<JSValue, ()> {
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
            let op_pos = self.pos;
            self.pos += 1;
            let rhs = self.parse_exponent()?;
            self.set_error_at(op_pos);
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

    pub(crate) fn parse_exponent(&mut self) -> Result<JSValue, ()> {
        let value = self.parse_unary()?;
        self.skip_ws();
        // Check for ** operator (right-associative)
        if self.peek() == Some(b'*') && self.peek_at(1) == Some(b'*') {
            let op_pos = self.pos;
            self.pos += 2;
            let rhs = self.parse_exponent()?;  // Right-associative recursion
            self.set_error_at(op_pos);
            self.pow_values(value, rhs)
        } else {
            Ok(value)
        }
    }

    pub(crate) fn parse_unary(&mut self) -> Result<JSValue, ()> {
        self.skip_ws();
        if self.starts_with_keyword(b"typeof") {
            let op_pos = self.pos;
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
            self.set_error_at(op_pos);
            return self.typeof_value(val);
        }
        if self.starts_with_keyword(b"void") {
            let op_pos = self.pos;
            self.pos += 4;
            self.skip_ws();
            let _ = self.parse_unary()?;
            self.set_error_at(op_pos);
            return Ok(Value::UNDEFINED);
        }
        if let Some(b'+') = self.peek() {
            let op_pos = self.pos;
            self.pos += 1;
            let val = self.parse_postfix()?;
            self.set_error_at(op_pos);
            return self.unary_plus(val);
        }
        if let Some(b'-') = self.peek() {
            let op_pos = self.pos;
            self.pos += 1;
            let val = self.parse_postfix()?;
            self.set_error_at(op_pos);
            return self.unary_minus(val);
        }
        if let Some(b'!') = self.peek() {
            if self.peek_at(1) != Some(b'=') {
                let op_pos = self.pos;
                self.pos += 1;
                let val = self.parse_postfix()?;
                self.set_error_at(op_pos);
                return self.logical_not(val);
            }
        }
        if let Some(b'~') = self.peek() {
            let op_pos = self.pos;
            self.pos += 1;
            let val = self.parse_postfix()?;
            self.set_error_at(op_pos);
            return self.bitwise_not(val);
        }
        self.parse_postfix()
    }

    pub(crate) fn parse_postfix(&mut self) -> Result<JSValue, ()> {
        let mut value = self.parse_primary()?;
        let mut this_val = Value::UNDEFINED;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'.') => {
                    let op_pos = self.pos;
                    self.pos += 1;
                    let rest = core::str::from_utf8(&self.input[self.pos..]).map_err(|_| ())?;
                    let (prop, remaining) = parse_identifier(rest).ok_or(())?;
                    let consumed = rest.len() - remaining.len();
                    self.pos += consumed;
                    this_val = value;
                    let ctx = unsafe { &mut *self.ctx };

                    if value.is_null() || value.is_undefined() {
                        self.set_error_at(op_pos);
                        js_throw_error(ctx, JSObjectClassEnum::TypeError, "cannot read property");
                        return Err(());
                    }

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
                    let op_pos = self.pos;
                    self.pos += 1;
                    let index = self.parse_expr()?;
                    self.skip_ws();
                    if self.peek() != Some(b']') {
                        return Err(());
                    }
                    self.pos += 1;
                    this_val = value;
                    let ctx = unsafe { &mut *self.ctx };
                    if value.is_null() || value.is_undefined() {
                        self.set_error_at(op_pos);
                        js_throw_error(ctx, JSObjectClassEnum::TypeError, "cannot read property");
                        return Err(());
                    }
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
                    let op_pos = self.pos;
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
                    self.set_error_at(op_pos);
                    value = self.call_builtin_method(ctx, value, this_val, &args)?;
                    this_val = Value::UNDEFINED;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    pub(crate) fn call_builtin_method(&mut self, ctx: &mut JSContextImpl, method: JSValue, this_val: JSValue, args: &[JSValue]) -> Result<JSValue, ()> {
        // Check if method is a builtin marker string.
        // Copy marker bytes to stack buffer to release ctx borrow (avoids heap alloc from .to_string()).
        let mut marker_buf = [0u8; 64];
        let marker_len;
        let is_marker;
        if let Some(bytes) = ctx.string_bytes(method) {
            let blen = bytes.len();
            if blen > 64 {
                // Not a recognized marker; try closure fallback
                let closure_marker = js_get_property_str(ctx, method, "__closure__");
                if closure_marker == Value::TRUE {
                    if let Some(val) = call_closure(ctx, method, args) {
                        return Ok(val);
                    }
                }
                return Err(());
            }
            marker_len = blen;
            marker_buf[..marker_len].copy_from_slice(bytes);
            is_marker = true;
        } else {
            marker_len = 0;
            is_marker = false;
        }
        if !is_marker {
            // Not a string; check if it's a closure (custom function)
            let closure_marker = js_get_property_str(ctx, method, "__closure__");
            if closure_marker == Value::TRUE {
                if let Some(val) = call_closure(ctx, method, args) {
                    return Ok(val);
                }
            }
            return Err(());
        }
        let marker: &str = match core::str::from_utf8(&marker_buf[..marker_len]) {
            Ok(s) => s,
            Err(_) => return Err(()),
        };
        {
                if marker == "__builtin_eval__" {
                    if let Some(val) = call_builtin_global_marker(ctx, marker, args) {
                        return Ok(val);
                    }
                }
                if marker == "__builtin_String__" {
                    if args.is_empty() {
                        return Ok(js_new_string(ctx, ""));
                    }
                    return Ok(js_to_string(ctx, args[0]));
                }
                if marker == "__builtin_Number__" {
                    if args.is_empty() {
                        return Ok(Value::from_int32(0));
                    }
                    let n = js_to_number(ctx, args[0]).unwrap_or(f64::NAN);
                    return Ok(number_to_value(ctx, n));
                }
                if marker == "__builtin_Boolean__" {
                    if args.is_empty() {
                        return Ok(Value::FALSE);
                    }
                    return Ok(Value::new_bool(crate::evals::is_truthy(ctx, args[0])));
                }
                if marker == "__builtin_Date_now__" {
                    return Ok(js_date_now(ctx));
                }
                if marker == "__builtin_console_log__" {
                    js_console_log(ctx, args);
                    return Ok(Value::UNDEFINED);
                }
                if marker == "__builtin_Object_toString__" {
                    return Ok(object_to_string_value(ctx, this_val));
                }
                if marker == "__builtin_Object_setPrototypeOf__" {
                    if args.len() < 2 {
                        js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.setPrototypeOf requires an object and prototype");
                        return Err(());
                    }
                    let target = args[0];
                    let proto = args[1];
                    if ctx.object_class_id(target).is_none() {
                        js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.setPrototypeOf called on non-object");
                        return Err(());
                    }
                    if !proto.is_null() && ctx.object_class_id(proto).is_none() {
                        js_throw_error(ctx, JSObjectClassEnum::TypeError, "Object.setPrototypeOf prototype must be object or null");
                        return Err(());
                    }
                    let _ = ctx.set_object_proto(target, proto);
                    return Ok(target);
                }
                // String methods
                if marker == "__builtin_string_charAt__" {
                    if args.len() == 1 {
                        if let Some(idx) = args[0].int32() {
                            if let Some(units) = string_utf16_units(ctx, this_val) {
                                if idx >= 0 && (idx as usize) < units.len() {
                                    let s = crate::evals::utf16_units_to_string_preserve_surrogates(&[units[idx as usize]]);
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
                            let slice_units = &units[start as usize..end as usize];
                            let s = crate::evals::utf16_units_to_string_preserve_surrogates(slice_units);
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
                            let slice_units = &units[start as usize..end as usize];
                            let s = crate::evals::utf16_units_to_string_preserve_surrogates(slice_units);
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
                            let slice_units = &units[start as usize..end as usize];
                            let s = crate::evals::utf16_units_to_string_preserve_surrogates(slice_units);
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
                            let slice_units = &units[start as usize..end as usize];
                            let s = crate::evals::utf16_units_to_string_preserve_surrogates(slice_units);
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
                } else if marker == "__builtin_string_replace__" {
                    if args.len() >= 2 {
                        let s = value_to_string(ctx, this_val);
                        if let Some((pattern, flags)) = regexp_parts(ctx, args[0]) {
                            let (re, global) = compile_regex(ctx, &pattern, &flags).map_err(|_| ())?;
                            let replaced = string_replace_regex(ctx, &s, &re, args[1], global);
                            return Ok(js_new_string(ctx, &replaced));
                        }
                        let search = value_to_string(ctx, args[0]);
                        let result = string_replace_nonregex(ctx, &s, &search, args[1], false);
                        return Ok(js_new_string(ctx, &result));
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
                            let replaced = string_replace_regex(ctx, &s, &re, args[1], true);
                            return Ok(js_new_string(ctx, &replaced));
                        }
                        let search = value_to_string(ctx, args[0]);
                        let result = string_replace_nonregex(ctx, &s, &search, args[1], true);
                        return Ok(js_new_string(ctx, &result));
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

                // JSON methods
                if marker == "__builtin_JSON_stringify__" {
                    if args.is_empty() {
                        return Ok(Value::UNDEFINED);
                    }
                    if let Some(json_str) = crate::json::json_stringify_value(ctx, args[0]) {
                        return Ok(js_new_string(ctx, &json_str));
                    }
                    return Ok(Value::UNDEFINED);
                } else if marker == "__builtin_JSON_parse__" {
                    if args.is_empty() {
                        js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "Unexpected end of JSON input");
                        return Err(());
                    }
                    if let Some(json_bytes) = ctx.string_bytes(args[0]) {
                        let json_str = core::str::from_utf8(json_bytes).unwrap_or("").to_string();
                        match parse_json(ctx, &json_str) {
                            Some(parsed_val) => return Ok(parsed_val),
                            None => {
                                js_throw_error(ctx, JSObjectClassEnum::SyntaxError, "Unexpected token in JSON");
                                return Err(());
                            }
                        }
                    } else {
                        js_throw_error(ctx, JSObjectClassEnum::TypeError, "Cannot convert to string");
                        return Err(());
                    }
                }

                // Error constructors
                if marker == "__builtin_Error__" || marker == "__builtin_TypeError__"
                    || marker == "__builtin_RangeError__" || marker == "__builtin_SyntaxError__"
                    || marker == "__builtin_ReferenceError__"
                {
                    let msg = if !args.is_empty() {
                        js_to_string(ctx, args[0])
                    } else {
                        js_new_string(ctx, "")
                    };
                    let class_id = match marker {
                        "__builtin_TypeError__" => JSObjectClassEnum::TypeError,
                        "__builtin_RangeError__" => JSObjectClassEnum::RangeError,
                        "__builtin_SyntaxError__" => JSObjectClassEnum::SyntaxError,
                        "__builtin_ReferenceError__" => JSObjectClassEnum::ReferenceError,
                        _ => JSObjectClassEnum::Error,
                    };
                    let err_obj = js_new_object(ctx);
                    js_set_property_str(ctx, err_obj, "message", msg);
                    // Set the class/tag on error object so toString works
                    let class_name = match class_id {
                        JSObjectClassEnum::TypeError => "TypeError",
                        JSObjectClassEnum::RangeError => "RangeError",
                        JSObjectClassEnum::SyntaxError => "SyntaxError",
                        JSObjectClassEnum::ReferenceError => "ReferenceError",
                        _ => "Error",
                    };
                    let name_val = js_new_string(ctx, class_name);
                    js_set_property_str(ctx, err_obj, "name", name_val);
                    return Ok(err_obj);
                }

                // Object static methods
                if marker == "__builtin_Object_keys__" {
                    if !args.is_empty() {
                        return Ok(js_object_keys(ctx, args[0]));
                    }
                    return Ok(js_new_array(ctx, 0));
                } else if marker == "__builtin_Array_isArray__" {
                    if args.len() == 1 {
                        if let Some(class_id) = ctx.object_class_id(args[0]) {
                            return Ok(Value::new_bool(class_id == JSObjectClassEnum::Array as u32));
                        }
                        return Ok(Value::FALSE);
                    }
                    return Ok(Value::FALSE);
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

    pub(crate) fn parse_primary(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_regex_literal(&mut self) -> Result<JSValue, ()> {
        let start_pos = self.pos;
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
            self.set_error_at(start_pos);
            return Err(());
        }
        Ok(val)
    }

    pub(crate) fn parse_identifier_value(&mut self) -> Result<JSValue, ()> {
        let start_pos = self.pos;
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
        let global = js_get_global_object(ctx);
        let val = js_get_property_str(ctx, global, name);
        if val.is_undefined() && !ctx.has_property_str(global, name.as_bytes()) {
            self.set_error_at(start_pos);
            js_throw_error(ctx, JSObjectClassEnum::ReferenceError, "not defined");
            return Err(());
        }
        Ok(val)
    }

    pub(crate) fn parse_string(&mut self) -> Result<JSValue, ()> {
        let quote = self.peek().ok_or(())?;
        self.pos += 1;
        let mut out = Vec::new();
        let mut escaped = false;
        while let Some(b) = self.peek() {
            self.pos += 1;
            if escaped {
                out.push(b);
                escaped = false;
                continue;
            }
            if b == quote {
                let raw = core::str::from_utf8(&out).map_err(|_| ())?;
                let unescaped = crate::evals::unescape_string_literal(raw);
                let ctx = unsafe { &mut *self.ctx };
                return Ok(js_new_string(ctx, &unescaped));
            }
            if b == b'\\' {
                out.push(b);
                escaped = true;
                continue;
            }
            out.push(b);
        }
        Err(())
    }

    pub(crate) fn parse_number_value(&mut self) -> Result<JSValue, ()> {
        let num = self.parse_number_raw()?;
        let ctx = unsafe { &mut *self.ctx };
        let val = number_to_value(ctx, num);
        if val.is_exception() {
            return Err(());
        }
        Ok(val)
    }

    pub(crate) fn parse_number_raw(&mut self) -> Result<f64, ()> {
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

    pub(crate) fn add_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
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

    pub(crate) fn sub_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln - rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    pub(crate) fn mul_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln * rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    pub(crate) fn div_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln / rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    pub(crate) fn mod_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln % rn);
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    pub(crate) fn pow_values(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_number(ctx, left).map_err(|_| ())?;
        let rn = js_to_number(ctx, right).map_err(|_| ())?;
        let val = number_to_value(ctx, ln.powf(rn));
        if val.is_exception() { Err(()) } else { Ok(val) }
    }

    pub(crate) fn unary_plus(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let n = js_to_number(ctx, val).map_err(|_| ())?;
        let out = number_to_value(ctx, n);
        if out.is_exception() { Err(()) } else { Ok(out) }
    }

    pub(crate) fn unary_minus(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let n = js_to_number(ctx, val).map_err(|_| ())?;
        let out = number_to_value(ctx, -n);
        if out.is_exception() { Err(()) } else { Ok(out) }
    }

    pub(crate) fn compare_values(&mut self, left: JSValue, right: JSValue, op: &[u8]) -> Result<JSValue, ()> {
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
                    } else if let Some(eq) = string_units_equal(ctx, left, right) {
                        eq
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
                    } else if let Some(eq) = string_units_equal(ctx, left, right) {
                        !eq
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
                    // Strict equality — no type coercion per ES spec
                    if left.0 == right.0 {
                        true
                    } else if let Some(eq) = string_units_equal(ctx, left, right) {
                        eq
                    } else if strict_eq_type_tag(ctx, left) != strict_eq_type_tag(ctx, right) {
                        // Different types → false (bool vs number, null vs undefined, etc.)
                        false
                    } else {
                        // Same type, different bit pattern — compare as numbers (int vs float)
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
                    // Strict inequality — no type coercion per ES spec
                    if left.0 == right.0 {
                        false
                    } else if let Some(eq) = string_units_equal(ctx, left, right) {
                        !eq
                    } else if strict_eq_type_tag(ctx, left) != strict_eq_type_tag(ctx, right) {
                        // Different types → true (not equal)
                        true
                    } else {
                        // Same type, different bit pattern — compare as numbers (int vs float)
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

    pub(crate) fn logical_and(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let left_truthy = self.is_truthy(left);
        if !left_truthy {
            Ok(left)
        } else {
            Ok(right)
        }
    }

    pub(crate) fn logical_or(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let left_truthy = self.is_truthy(left);
        if left_truthy {
            Ok(left)
        } else {
            Ok(right)
        }
    }

    pub(crate) fn logical_not(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let truthy = self.is_truthy(val);
        Ok(if truthy { Value::FALSE } else { Value::TRUE })
    }

    pub(crate) fn bitwise_and(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_int32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln & rn))
    }

    pub(crate) fn bitwise_or(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_int32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln | rn))
    }

    pub(crate) fn bitwise_xor(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_int32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln ^ rn))
    }

    pub(crate) fn bitwise_not(&mut self, val: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let n = js_to_int32(ctx, val).map_err(|_| ())?;
        Ok(Value::from_int32(!n))
    }

    pub(crate) fn left_shift(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_uint32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln << (rn & 0x1f)))
    }

    pub(crate) fn right_shift(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
        let ctx = unsafe { &mut *self.ctx };
        let ln = js_to_int32(ctx, left).map_err(|_| ())?;
        let rn = js_to_uint32(ctx, right).map_err(|_| ())?;
        Ok(Value::from_int32(ln >> (rn & 0x1f)))
    }

    pub(crate) fn unsigned_right_shift(&mut self, left: JSValue, right: JSValue) -> Result<JSValue, ()> {
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

    pub(crate) fn is_truthy(&self, val: JSValue) -> bool {
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

    pub(crate) fn peek_at(&self, offset: usize) -> Option<u8> {
        self.input.get(self.pos + offset).copied()
    }

    pub(crate) fn starts_with_keyword(&self, kw: &[u8]) -> bool {
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

    pub(crate) fn typeof_value(&self, val: JSValue) -> Result<JSValue, ()> {
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

    pub(crate) fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    pub(crate) fn parse_array_literal(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn parse_object_literal(&mut self) -> Result<JSValue, ()> {
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

    pub(crate) fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
}
