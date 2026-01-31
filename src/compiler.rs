//! Bytecode compiler scaffolding.
#![allow(dead_code)]

use crate::bytecode::{BytecodeFunction, BytecodeModule, Instruction, OpCode};
use crate::helpers::{is_simple_string_literal, number_to_value};
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

        let mut expr = ExprCompiler::new(ctx, s);
        expr.parse_expr()?;
        expr.skip_ws();
        if expr.pos != expr.input.len() {
            return Err(CompileError {
                message: "unsupported bytecode expression".to_string(),
            });
        }
        expr.func.code.push(Instruction { op: OpCode::Return, a: 0, b: 0, c: 0 });
        Ok(BytecodeModule::new(expr.func))
    }

    pub fn compile_empty(&mut self) -> BytecodeModule {
        BytecodeModule::new(BytecodeFunction::new(None))
    }
}

struct ExprCompiler<'a> {
    ctx: &'a mut JSContextImpl,
    input: &'a [u8],
    pos: usize,
    func: BytecodeFunction,
}

impl<'a> ExprCompiler<'a> {
    fn new(ctx: &'a mut JSContextImpl, src: &'a str) -> Self {
        Self {
            ctx,
            input: src.as_bytes(),
            pos: 0,
            func: BytecodeFunction::new(None),
        }
    }

    fn parse_expr(&mut self) -> Result<(), CompileError> {
        self.parse_term()?;
        loop {
            self.skip_ws();
            if self.consume(b'+') {
                self.parse_term()?;
                self.emit_op(OpCode::Add);
            } else if self.consume(b'-') {
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
            self.emit_const(0.0);
            self.parse_factor()?;
            self.emit_op(OpCode::Sub);
            return Ok(());
        }
        if self.consume(b'(') {
            self.parse_expr()?;
            self.skip_ws();
            if !self.consume(b')') {
                return Err(CompileError { message: "missing ')'".to_string() });
            }
            return Ok(());
        }
        let num = self.parse_number()?;
        self.emit_const(num);
        Ok(())
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

    fn consume_digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }

    fn emit_const(&mut self, num: f64) {
        let value = number_to_value(self.ctx, num);
        let idx = self.func.constants.len();
        self.func.constants.push(value);
        self.func.code.push(Instruction { op: OpCode::Const, a: idx as u32, b: 0, c: 0 });
    }

    fn emit_op(&mut self, op: OpCode) {
        self.func.code.push(Instruction { op, a: 0, b: 0, c: 0 });
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn consume(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }
}
