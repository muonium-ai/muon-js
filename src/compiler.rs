//! Bytecode compiler for statements and expressions.
#![allow(dead_code)]

use crate::bytecode::{BytecodeFunction, BytecodeModule, Instruction, OpCode};
use crate::helpers::{is_simple_string_literal, number_to_value, is_ident_start};
use crate::JSContextImpl;

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
}

pub struct Compiler;

impl Compiler {
    pub fn new() -> Self {
        Self
    }

    pub fn compile_program(&mut self, ctx: &mut JSContextImpl, src: &str) -> Result<BytecodeModule, CompileError> {
        let s = src.trim();
        if is_simple_string_literal(s) {
            let mut func = BytecodeFunction::new(None);
            let inner = &s[1..s.len() - 1];
            let value = crate::api::js_new_string(ctx, inner);
            func.constants.push(value);
            func.code.push(Instruction { op: OpCode::Const, a: 0, b: 0, c: 0 });
            func.code.push(Instruction { op: OpCode::Return, a: 0, b: 0, c: 0 });
            return Ok(BytecodeModule::new(func));
        }

        let mut sc = StmtCompiler::new(ctx);
        let mut expr = ExprCompiler::new_from_stmt(&mut sc, s);
        expr.parse_comma()?;
        expr.skip_ws();
        if expr.pos != expr.input.len() {
            return Err(CompileError {
                message: "unsupported bytecode expression".to_string(),
            });
        }
        sc.func.code.push(Instruction { op: OpCode::Return, a: 0, b: 0, c: 0 });
        Ok(BytecodeModule::new(sc.func))
    }

    pub fn compile_empty(&mut self) -> BytecodeModule {
        BytecodeModule::new(BytecodeFunction::new(None))
    }
}

// ============================================================
// Statement compiler — compiles a list of statements to bytecode
// ============================================================

pub struct StmtCompiler<'a> {
    ctx: &'a mut JSContextImpl,
    func: BytecodeFunction,
}

impl<'a> StmtCompiler<'a> {
    pub fn new(ctx: &'a mut JSContextImpl) -> Self {
        Self {
            ctx,
            func: BytecodeFunction::new(None),
        }
    }

    /// Pre-register parameter names as local slots (must be called before compile_stmts).
    pub fn add_params(&mut self, param_names: &[String]) {
        for name in param_names {
            self.ensure_local(name);
        }
    }

    /// Try to compile a list of pre-parsed statements to bytecode.
    pub fn compile_stmts(mut self, stmts: &[String]) -> Result<BytecodeModule, CompileError> {
        for stmt in stmts {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                continue;
            }
            self.compile_stmt(trimmed)?;
        }
        // Implicit return undefined at end
        let undef_idx = self.add_undefined_const();
        self.func.code.push(Instruction { op: OpCode::Const, a: undef_idx, b: 0, c: 0 });
        self.func.code.push(Instruction { op: OpCode::Return, a: 0, b: 0, c: 0 });
        Ok(BytecodeModule::new(self.func))
    }

    fn compile_stmt(&mut self, stmt: &str) -> Result<(), CompileError> {
        let s = stmt.trim();
        if s.is_empty() {
            return Ok(());
        }

        // var declaration
        if s.starts_with("var ") {
            return self.compile_var_decl(&s[4..]);
        }

        // return statement
        if s == "return" {
            let undef_idx = self.add_undefined_const();
            self.func.code.push(Instruction { op: OpCode::Const, a: undef_idx, b: 0, c: 0 });
            self.func.code.push(Instruction { op: OpCode::Return, a: 0, b: 0, c: 0 });
            return Ok(());
        }
        if s.starts_with("return ") || s.starts_with("return\t") {
            let expr = s[7..].trim();
            if expr.is_empty() {
                let undef_idx = self.add_undefined_const();
                self.func.code.push(Instruction { op: OpCode::Const, a: undef_idx, b: 0, c: 0 });
            } else {
                self.compile_full_expr(expr)?;
            }
            self.func.code.push(Instruction { op: OpCode::Return, a: 0, b: 0, c: 0 });
            return Ok(());
        }

        // for loop
        if s.starts_with("for ") || s.starts_with("for(") {
            return self.compile_for_loop(s);
        }

        // if statement
        if s.starts_with("if ") || s.starts_with("if(") {
            return self.compile_if_stmt(s);
        }

        // Expression statement (discard result)
        self.compile_full_expr(s)?;
        self.func.code.push(Instruction { op: OpCode::Drop, a: 0, b: 0, c: 0 });
        Ok(())
    }

    fn compile_var_decl(&mut self, rest: &str) -> Result<(), CompileError> {
        let rest = rest.trim();
        if let Some(eq_pos) = find_assignment_eq(rest) {
            let name = rest[..eq_pos].trim();
            let expr = rest[eq_pos + 1..].trim();
            if !is_valid_identifier(name) {
                return Err(CompileError { message: format!("invalid var name: {}", name) });
            }
            let slot = self.ensure_local(name);
            self.compile_full_expr(expr)?;
            self.func.code.push(Instruction { op: OpCode::StoreLocal, a: slot, b: 0, c: 0 });
            self.func.code.push(Instruction { op: OpCode::Drop, a: 0, b: 0, c: 0 });
        } else {
            let name = rest.trim_end_matches(';').trim();
            if !is_valid_identifier(name) {
                return Err(CompileError { message: format!("invalid var name: {}", name) });
            }
            let _slot = self.ensure_local(name);
        }
        Ok(())
    }

    fn compile_for_loop(&mut self, s: &str) -> Result<(), CompileError> {
        let rest = s.strip_prefix("for").unwrap().trim();
        let rest = rest.strip_prefix('(')
            .ok_or_else(|| CompileError { message: "expected '(' after for".to_string() })?;

        let close = find_matching_paren(rest)
            .ok_or_else(|| CompileError { message: "unmatched '(' in for".to_string() })?;

        let header = &rest[..close];
        let after = rest[close + 1..].trim();

        let parts = split_for_header(header)?;
        let (init, cond, update) = (&parts[0], &parts[1], &parts[2]);

        // Compile init
        let init = init.trim();
        if !init.is_empty() {
            if init.starts_with("var ") {
                self.compile_var_decl(&init[4..])?;
            } else {
                self.compile_full_expr(init)?;
                self.func.code.push(Instruction { op: OpCode::Drop, a: 0, b: 0, c: 0 });
            }
        }

        let loop_start = self.func.code.len() as u32;

        // Compile condition
        let cond = cond.trim();
        let jump_end;
        if !cond.is_empty() {
            self.compile_full_expr(cond)?;
            jump_end = self.func.code.len();
            self.func.code.push(Instruction { op: OpCode::JumpIfFalse, a: 0, b: 0, c: 0 });
        } else {
            jump_end = usize::MAX;
        }

        // Compile body
        let body = extract_braced_body(after)?;
        let body_stmts = simple_split_statements(&body)?;
        for bs in &body_stmts {
            self.compile_stmt(bs)?;
        }

        // Compile update
        let update = update.trim();
        if !update.is_empty() {
            if !self.try_compile_inc_update(update)? {
                self.compile_full_expr(update)?;
                self.func.code.push(Instruction { op: OpCode::Drop, a: 0, b: 0, c: 0 });
            }
        }

        // Jump back to loop start
        self.func.code.push(Instruction { op: OpCode::Jump, a: loop_start, b: 0, c: 0 });

        // Patch jump_end
        let end_pc = self.func.code.len() as u32;
        if jump_end != usize::MAX {
            self.func.code[jump_end].a = end_pc;
        }

        Ok(())
    }

    fn try_compile_inc_update(&mut self, update: &str) -> Result<bool, CompileError> {
        let update = update.trim();
        if let Some(pos) = update.find("+=") {
            let name = update[..pos].trim();
            let val_str = update[pos + 2..].trim();
            if is_valid_identifier(name) {
                if let Some(slot) = self.find_local(name) {
                    if let Ok(n) = val_str.parse::<i32>() {
                        self.func.code.push(Instruction {
                            op: OpCode::IncLocal,
                            a: slot,
                            b: n as u32,
                            c: 0,
                        });
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    fn compile_if_stmt(&mut self, s: &str) -> Result<(), CompileError> {
        let rest = s.strip_prefix("if").unwrap().trim();
        let rest = rest.strip_prefix('(')
            .ok_or_else(|| CompileError { message: "expected '(' after if".to_string() })?;

        let close = find_matching_paren(rest)
            .ok_or_else(|| CompileError { message: "unmatched '(' in if".to_string() })?;

        let cond = &rest[..close];
        let after = rest[close + 1..].trim();

        self.compile_full_expr(cond)?;

        let jump_else = self.func.code.len();
        self.func.code.push(Instruction { op: OpCode::JumpIfFalse, a: 0, b: 0, c: 0 });

        let body = extract_braced_body(after)?;
        let body_stmts = simple_split_statements(&body)?;
        for bs in &body_stmts {
            self.compile_stmt(bs)?;
        }

        // Check for else
        if let Some(else_start) = find_else_after_brace(after) {
            let else_part = after[else_start..].trim();
            let jump_end = self.func.code.len();
            self.func.code.push(Instruction { op: OpCode::Jump, a: 0, b: 0, c: 0 });

            self.func.code[jump_else].a = self.func.code.len() as u32;

            if else_part.starts_with("if ") || else_part.starts_with("if(") {
                self.compile_if_stmt(else_part)?;
            } else {
                let else_body = extract_braced_body(else_part)?;
                let else_stmts = simple_split_statements(&else_body)?;
                for es in &else_stmts {
                    self.compile_stmt(es)?;
                }
            }

            self.func.code[jump_end].a = self.func.code.len() as u32;
        } else {
            self.func.code[jump_else].a = self.func.code.len() as u32;
        }

        Ok(())
    }

    fn compile_full_expr(&mut self, src: &str) -> Result<(), CompileError> {
        let mut ec = ExprCompiler::new_from_stmt(self, src);
        ec.parse_comma()?;
        ec.skip_ws();
        if ec.pos != ec.input.len() {
            return Err(CompileError {
                message: format!("unexpected trailing: '{}'", &src[ec.pos..]),
            });
        }
        Ok(())
    }

    fn ensure_local(&mut self, name: &str) -> u32 {
        if let Some(idx) = self.func.locals.iter().position(|n| n == name) {
            return idx as u32;
        }
        let idx = self.func.locals.len() as u32;
        self.func.locals.push(name.to_string());
        idx
    }

    fn find_local(&self, name: &str) -> Option<u32> {
        self.func.locals.iter().position(|n| n == name).map(|i| i as u32)
    }

    fn add_undefined_const(&mut self) -> u32 {
        for (i, v) in self.func.constants.iter().enumerate() {
            if v.is_undefined() {
                return i as u32;
            }
        }
        let idx = self.func.constants.len() as u32;
        self.func.constants.push(crate::types::JSValue::UNDEFINED);
        idx
    }

    fn add_number_const(&mut self, n: f64) -> u32 {
        let value = number_to_value(self.ctx, n);
        let idx = self.func.constants.len() as u32;
        self.func.constants.push(value);
        idx
    }

    fn add_string_const(&mut self, s: &str) -> u32 {
        let value = crate::api::js_new_string(self.ctx, s);
        let idx = self.func.constants.len() as u32;
        self.func.constants.push(value);
        idx
    }
}

// ============================================================
// Expression compiler (used by StmtCompiler)
// ============================================================

struct ExprCompiler<'a, 'b> {
    sc: &'a mut StmtCompiler<'b>,
    input: &'a [u8],
    pos: usize,
}

impl<'a, 'b> ExprCompiler<'a, 'b> {
    fn new_from_stmt(sc: &'a mut StmtCompiler<'b>, src: &'a str) -> Self {
        Self {
            sc,
            input: src.as_bytes(),
            pos: 0,
        }
    }

    fn emit_op(&mut self, op: OpCode) {
        self.sc.func.code.push(Instruction { op, a: 0, b: 0, c: 0 });
    }

    fn emit_inst(&mut self, op: OpCode, a: u32) {
        self.sc.func.code.push(Instruction { op, a, b: 0, c: 0 });
    }

    fn emit_const(&mut self, num: f64) {
        let idx = self.sc.add_number_const(num);
        self.sc.func.code.push(Instruction { op: OpCode::Const, a: idx, b: 0, c: 0 });
    }

    fn emit_string_const(&mut self, s: &str) {
        let idx = self.sc.add_string_const(s);
        self.sc.func.code.push(Instruction { op: OpCode::Const, a: idx, b: 0, c: 0 });
    }

    fn emit_jump(&mut self, op: OpCode) -> usize {
        let idx = self.sc.func.code.len();
        self.sc.func.code.push(Instruction { op, a: 0, b: 0, c: 0 });
        idx
    }

    fn patch_jump(&mut self, at: usize) {
        let target = self.sc.func.code.len() as u32;
        if let Some(ins) = self.sc.func.code.get_mut(at) {
            ins.a = target;
        }
    }

    fn parse_comma(&mut self) -> Result<(), CompileError> {
        self.parse_conditional()?;
        loop {
            self.skip_ws();
            if self.consume(b',') {
                self.emit_op(OpCode::Drop);
                self.parse_conditional()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_conditional(&mut self) -> Result<(), CompileError> {
        self.parse_logical_or()?;
        self.skip_ws();
        if !self.consume(b'?') {
            return Ok(());
        }
        let jump_if_false = self.emit_jump(OpCode::JumpIfFalse);
        self.parse_conditional()?;
        let jump_end = self.emit_jump(OpCode::Jump);
        self.patch_jump(jump_if_false);
        self.skip_ws();
        if !self.consume(b':') {
            return Err(CompileError { message: "missing ':'".to_string() });
        }
        self.parse_conditional()?;
        self.patch_jump(jump_end);
        Ok(())
    }

    fn parse_logical_or(&mut self) -> Result<(), CompileError> {
        self.parse_logical_and()?;
        loop {
            self.skip_ws();
            if self.consume_seq(b"||") {
                self.parse_logical_and()?;
                self.emit_op(OpCode::Or);
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_logical_and(&mut self) -> Result<(), CompileError> {
        self.parse_comparison()?;
        loop {
            self.skip_ws();
            if self.consume_seq(b"&&") {
                self.parse_comparison()?;
                self.emit_op(OpCode::And);
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_comparison(&mut self) -> Result<(), CompileError> {
        self.parse_assignment()?;
        loop {
            self.skip_ws();
            if self.consume_seq(b"===") {
                self.parse_assignment()?;
                self.emit_op(OpCode::StrictEq);
            } else if self.consume_seq(b"!==") {
                self.parse_assignment()?;
                self.emit_op(OpCode::StrictNeq);
            } else if self.consume_seq(b"==") {
                self.parse_assignment()?;
                self.emit_op(OpCode::Eq);
            } else if self.consume_seq(b"!=") {
                self.parse_assignment()?;
                self.emit_op(OpCode::Neq);
            } else if self.consume_seq(b"<=") {
                self.parse_assignment()?;
                self.emit_op(OpCode::Le);
            } else if self.consume_seq(b">=") {
                self.parse_assignment()?;
                self.emit_op(OpCode::Ge);
            } else if self.consume(b'<') {
                self.parse_assignment()?;
                self.emit_op(OpCode::Lt);
            } else if self.consume(b'>') {
                self.parse_assignment()?;
                self.emit_op(OpCode::Gt);
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_assignment(&mut self) -> Result<(), CompileError> {
        self.skip_ws();
        let start = self.pos;

        if let Some(name) = self.parse_identifier() {
            self.skip_ws();

            // Compound assignments: +=, -=, *=
            if self.consume_seq(b"+=") {
                if let Some(slot) = self.sc.find_local(&name) {
                    self.sc.func.code.push(Instruction { op: OpCode::LoadLocal, a: slot, b: 0, c: 0 });
                    self.parse_expr()?;
                    self.emit_op(OpCode::Add);
                    self.sc.func.code.push(Instruction { op: OpCode::Dup, a: 0, b: 0, c: 0 });
                    self.sc.func.code.push(Instruction { op: OpCode::StoreLocal, a: slot, b: 0, c: 0 });
                    self.emit_op(OpCode::Drop);
                    return Ok(());
                }
                let name_idx = self.sc.add_string_const(&name);
                self.sc.func.code.push(Instruction { op: OpCode::LoadGlobal, a: name_idx, b: 0, c: 0 });
                self.parse_expr()?;
                self.emit_op(OpCode::Add);
                let name_idx2 = self.sc.add_string_const(&name);
                self.sc.func.code.push(Instruction { op: OpCode::StoreGlobal, a: name_idx2, b: 0, c: 0 });
                return Ok(());
            }
            if self.consume_seq(b"-=") {
                if let Some(slot) = self.sc.find_local(&name) {
                    self.sc.func.code.push(Instruction { op: OpCode::LoadLocal, a: slot, b: 0, c: 0 });
                    self.parse_expr()?;
                    self.emit_op(OpCode::Sub);
                    self.sc.func.code.push(Instruction { op: OpCode::Dup, a: 0, b: 0, c: 0 });
                    self.sc.func.code.push(Instruction { op: OpCode::StoreLocal, a: slot, b: 0, c: 0 });
                    self.emit_op(OpCode::Drop);
                    return Ok(());
                }
                return Err(CompileError { message: "unsupported global -=".to_string() });
            }

            // Simple assignment
            if self.consume(b'=') {
                if self.peek() == Some(b'=') {
                    self.pos = start;
                    return self.parse_expr();
                }
                self.parse_assignment()?;
                if let Some(slot) = self.sc.find_local(&name) {
                    self.sc.func.code.push(Instruction { op: OpCode::Dup, a: 0, b: 0, c: 0 });
                    self.sc.func.code.push(Instruction { op: OpCode::StoreLocal, a: slot, b: 0, c: 0 });
                    self.emit_op(OpCode::Drop);
                } else {
                    let name_idx = self.sc.add_string_const(&name);
                    self.sc.func.code.push(Instruction { op: OpCode::StoreGlobal, a: name_idx, b: 0, c: 0 });
                }
                return Ok(());
            }
        }
        self.pos = start;
        self.parse_expr()
    }

    fn parse_expr(&mut self) -> Result<(), CompileError> {
        self.parse_term()?;
        loop {
            self.skip_ws();
            if self.consume(b'+') {
                if self.peek() == Some(b'+') {
                    return Err(CompileError { message: "++ not supported in bytecode".to_string() });
                }
                self.parse_term()?;
                self.emit_op(OpCode::Add);
            } else if self.consume(b'-') {
                if self.peek() == Some(b'-') {
                    return Err(CompileError { message: "-- not supported in bytecode".to_string() });
                }
                self.parse_term()?;
                self.emit_op(OpCode::Sub);
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_term(&mut self) -> Result<(), CompileError> {
        self.parse_factor()?;
        loop {
            self.skip_ws();
            if self.consume(b'*') {
                self.parse_factor()?;
                self.emit_op(OpCode::Mul);
            } else if self.consume(b'/') {
                self.parse_factor()?;
                self.emit_op(OpCode::Div);
            } else if self.consume(b'%') {
                self.parse_factor()?;
                self.emit_op(OpCode::Mod);
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_factor(&mut self) -> Result<(), CompileError> {
        self.skip_ws();
        if self.consume(b'+') {
            return self.parse_factor();
        }
        if self.consume(b'-') {
            self.parse_factor()?;
            self.emit_op(OpCode::Neg);
            return Ok(());
        }
        if self.consume(b'!') {
            self.parse_factor()?;
            self.emit_op(OpCode::Not);
            return Ok(());
        }
        if self.consume(b'~') {
            self.parse_factor()?;
            self.emit_op(OpCode::BitNot);
            return Ok(());
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<(), CompileError> {
        self.parse_primary()?;
        loop {
            self.skip_ws();
            if self.consume(b'.') {
                let prop = self.parse_identifier()
                    .ok_or_else(|| CompileError { message: "expected property name after '.'".to_string() })?;
                self.skip_ws();
                if self.check(b'(') {
                    // Method call: obj.method(args) — preserve `this` (obj) for CallMethod
                    self.sc.func.code.push(Instruction { op: OpCode::Dup, a: 0, b: 0, c: 0 });
                    self.emit_string_const(&prop);
                    self.emit_op(OpCode::GetProp);
                    // Now parse the call arguments
                    self.consume(b'(');
                    let mut argc: u32 = 0;
                    self.skip_ws();
                    if !self.check(b')') {
                        self.parse_conditional()?;
                        argc += 1;
                        loop {
                            self.skip_ws();
                            if self.consume(b',') {
                                self.parse_conditional()?;
                                argc += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    self.skip_ws();
                    if !self.consume(b')') {
                        return Err(CompileError { message: "expected ')'".to_string() });
                    }
                    self.sc.func.code.push(Instruction { op: OpCode::CallMethod, a: argc, b: 0, c: 0 });
                } else {
                    // Property access: obj.prop
                    self.emit_string_const(&prop);
                    self.emit_op(OpCode::GetProp);
                }
            } else if self.consume(b'[') {
                self.parse_comma()?;
                self.skip_ws();
                if !self.consume(b']') {
                    return Err(CompileError { message: "expected ']'".to_string() });
                }
                self.emit_op(OpCode::GetElem);
            } else if self.consume(b'(') {
                let mut argc: u32 = 0;
                self.skip_ws();
                if !self.check(b')') {
                    self.parse_conditional()?;
                    argc += 1;
                    loop {
                        self.skip_ws();
                        if self.consume(b',') {
                            self.parse_conditional()?;
                            argc += 1;
                        } else {
                            break;
                        }
                    }
                }
                self.skip_ws();
                if !self.consume(b')') {
                    return Err(CompileError { message: "expected ')'".to_string() });
                }
                self.sc.func.code.push(Instruction { op: OpCode::Call, a: argc, b: 0, c: 0 });
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_primary(&mut self) -> Result<(), CompileError> {
        self.skip_ws();

        if self.consume(b'(') {
            self.parse_comma()?;
            self.skip_ws();
            if !self.consume(b')') {
                return Err(CompileError { message: "expected ')'".to_string() });
            }
            return Ok(());
        }

        if self.peek() == Some(b'\'') || self.peek() == Some(b'"') {
            let s = self.parse_string_literal()?;
            self.emit_string_const(&s);
            return Ok(());
        }

        if matches!(self.peek(), Some(b'0'..=b'9') | Some(b'.')) {
            let num = self.parse_number()?;
            self.emit_const(num);
            return Ok(());
        }

        if let Some(name) = self.parse_identifier() {
            match name.as_str() {
                "true" => {
                    let idx = self.sc.func.constants.len() as u32;
                    self.sc.func.constants.push(crate::types::JSValue::TRUE);
                    self.sc.func.code.push(Instruction { op: OpCode::Const, a: idx, b: 0, c: 0 });
                }
                "false" => {
                    let idx = self.sc.func.constants.len() as u32;
                    self.sc.func.constants.push(crate::types::JSValue::FALSE);
                    self.sc.func.code.push(Instruction { op: OpCode::Const, a: idx, b: 0, c: 0 });
                }
                "null" => {
                    let idx = self.sc.func.constants.len() as u32;
                    self.sc.func.constants.push(crate::types::JSValue::NULL);
                    self.sc.func.code.push(Instruction { op: OpCode::Const, a: idx, b: 0, c: 0 });
                }
                "undefined" => {
                    let idx = self.sc.add_undefined_const();
                    self.sc.func.code.push(Instruction { op: OpCode::Const, a: idx, b: 0, c: 0 });
                }
                "Number" => {
                    self.skip_ws();
                    if self.consume(b'(') {
                        self.parse_comma()?;
                        self.skip_ws();
                        if !self.consume(b')') {
                            return Err(CompileError { message: "expected ')'".to_string() });
                        }
                        self.emit_op(OpCode::ToNumber);
                        return Ok(());
                    }
                    let name_idx = self.sc.add_string_const("Number");
                    self.sc.func.code.push(Instruction { op: OpCode::LoadGlobal, a: name_idx, b: 0, c: 0 });
                }
                _ => {
                    if let Some(slot) = self.sc.find_local(&name) {
                        self.sc.func.code.push(Instruction { op: OpCode::LoadLocal, a: slot, b: 0, c: 0 });
                    } else {
                        let name_idx = self.sc.add_string_const(&name);
                        self.sc.func.code.push(Instruction { op: OpCode::LoadGlobal, a: name_idx, b: 0, c: 0 });
                    }
                }
            }
            return Ok(());
        }

        Err(CompileError { message: format!("unexpected token at pos {}", self.pos) })
    }

    fn parse_string_literal(&mut self) -> Result<String, CompileError> {
        let quote = self.input[self.pos];
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.input.len() {
            let b = self.input[self.pos];
            if b == quote {
                self.pos += 1;
                return Ok(s);
            }
            if b == b'\\' && self.pos + 1 < self.input.len() {
                self.pos += 1;
                match self.input[self.pos] {
                    b'n' => s.push('\n'),
                    b'r' => s.push('\r'),
                    b't' => s.push('\t'),
                    b'\\' => s.push('\\'),
                    b'\'' => s.push('\''),
                    b'"' => s.push('"'),
                    b'0' => s.push('\0'),
                    other => { s.push('\\'); s.push(other as char); }
                }
                self.pos += 1;
                continue;
            }
            s.push(b as char);
            self.pos += 1;
        }
        Err(CompileError { message: "unterminated string".to_string() })
    }

    fn parse_number(&mut self) -> Result<f64, CompileError> {
        self.skip_ws();
        let start = self.pos;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.consume_digits();
        } else {
            self.consume_digits();
            if self.peek() == Some(b'.') {
                self.pos += 1;
                self.consume_digits();
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            self.consume_digits();
        }
        if self.pos == start {
            return Err(CompileError { message: "expected number".to_string() });
        }
        let slice = &self.input[start..self.pos];
        let s = core::str::from_utf8(slice).map_err(|_| CompileError { message: "invalid number".to_string() })?;
        let n = s.parse::<f64>().map_err(|_| CompileError { message: "invalid number".to_string() })?;
        Ok(n)
    }

    fn parse_identifier(&mut self) -> Option<String> {
        self.skip_ws();
        let start = self.pos;
        let first = self.peek()?;
        if !is_ident_start(first) {
            return None;
        }
        self.pos += 1;
        while let Some(b) = self.peek() {
            if is_ident_start(b) || (b'0'..=b'9').contains(&b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let slice = &self.input[start..self.pos];
        core::str::from_utf8(slice).ok().map(|s| s.to_string())
    }

    fn consume_digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn consume(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) { self.pos += 1; true } else { false }
    }

    fn check(&self, b: u8) -> bool {
        self.peek() == Some(b)
    }

    fn consume_seq(&mut self, seq: &[u8]) -> bool {
        if self.input.len().saturating_sub(self.pos) < seq.len() {
            return false;
        }
        if self.input[self.pos..self.pos + seq.len()] == *seq {
            self.pos += seq.len();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
}

// ============================================================
// Helper functions
// ============================================================

fn is_valid_identifier(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return false; }
    if !is_ident_start(bytes[0]) { return false; }
    bytes[1..].iter().all(|&b| is_ident_start(b) || (b'0'..=b'9').contains(&b))
}

fn find_assignment_eq(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'=' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' { continue; }
            if i > 0 && matches!(bytes[i - 1], b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/') { continue; }
            return Some(i);
        }
    }
    None
}

fn find_matching_paren(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_sq { if b == b'\'' && (i == 0 || bytes[i - 1] != b'\\') { in_sq = false; } continue; }
        if in_dq { if b == b'"' && (i == 0 || bytes[i - 1] != b'\\') { in_dq = false; } continue; }
        match b {
            b'\'' => in_sq = true,
            b'"' => in_dq = true,
            b'(' => depth += 1,
            b')' => { if depth == 0 { return Some(i); } depth -= 1; }
            _ => {}
        }
    }
    None
}

fn extract_braced_body(s: &str) -> Result<String, CompileError> {
    let s = s.trim();
    if !s.starts_with('{') {
        return Err(CompileError { message: "expected '{'".to_string() });
    }
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_sq { if b == b'\'' && (i == 0 || bytes[i - 1] != b'\\') { in_sq = false; } continue; }
        if in_dq { if b == b'"' && (i == 0 || bytes[i - 1] != b'\\') { in_dq = false; } continue; }
        match b {
            b'\'' => in_sq = true,
            b'"' => in_dq = true,
            b'{' => depth += 1,
            b'}' => { depth -= 1; if depth == 0 { return Ok(s[1..i].to_string()); } }
            _ => {}
        }
    }
    Err(CompileError { message: "unmatched '{'".to_string() })
}

fn simple_split_statements(body: &str) -> Result<Vec<String>, CompileError> {
    let mut stmts = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut start = 0;
    let mut depth_paren = 0i32;
    let mut depth_brace = 0i32;
    let mut depth_bracket = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;

    while i < bytes.len() {
        let b = bytes[i];
        if in_sq { if b == b'\'' && (i == 0 || bytes[i - 1] != b'\\') { in_sq = false; } i += 1; continue; }
        if in_dq { if b == b'"' && (i == 0 || bytes[i - 1] != b'\\') { in_dq = false; } i += 1; continue; }
        match b {
            b'\'' => { in_sq = true; i += 1; }
            b'"' => { in_dq = true; i += 1; }
            b'(' => { depth_paren += 1; i += 1; }
            b')' => { depth_paren -= 1; i += 1; }
            b'{' => { depth_brace += 1; i += 1; }
            b'}' => {
                depth_brace -= 1;
                i += 1;
                if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 {
                    let s = body[start..i].trim();
                    if !s.is_empty() { stmts.push(s.to_string()); }
                    start = i;
                }
            }
            b'[' => { depth_bracket += 1; i += 1; }
            b']' => { depth_bracket -= 1; i += 1; }
            b';' | b'\n' if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 => {
                let s = body[start..i].trim();
                if !s.is_empty() { stmts.push(s.to_string()); }
                i += 1;
                start = i;
            }
            _ => { i += 1; }
        }
    }
    let s = body[start..].trim();
    if !s.is_empty() { stmts.push(s.to_string()); }
    Ok(stmts)
}

fn split_for_header(header: &str) -> Result<[String; 3], CompileError> {
    let bytes = header.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_sq { if b == b'\'' && (i == 0 || bytes[i - 1] != b'\\') { in_sq = false; } continue; }
        if in_dq { if b == b'"' && (i == 0 || bytes[i - 1] != b'\\') { in_dq = false; } continue; }
        match b {
            b'\'' => in_sq = true,
            b'"' => in_dq = true,
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth == 0 => { parts.push(header[start..i].to_string()); start = i + 1; }
            _ => {}
        }
    }
    parts.push(header[start..].to_string());
    if parts.len() != 3 {
        return Err(CompileError { message: format!("for header must have 3 parts, got {}", parts.len()) });
    }
    Ok([parts[0].clone(), parts[1].clone(), parts[2].clone()])
}

fn find_else_after_brace(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_sq { if b == b'\'' && (i == 0 || bytes[i - 1] != b'\\') { in_sq = false; } continue; }
        if in_dq { if b == b'"' && (i == 0 || bytes[i - 1] != b'\\') { in_dq = false; } continue; }
        match b {
            b'\'' => in_sq = true,
            b'"' => in_dq = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let after = s[i + 1..].trim_start();
                    if after.starts_with("else") {
                        let else_pos = s.len() - after.len();
                        let after_else = &after[4..];
                        if after_else.is_empty() || after_else.starts_with(' ') || after_else.starts_with('{') || after_else.starts_with('\n') {
                            return Some(else_pos + 4);
                        }
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}
