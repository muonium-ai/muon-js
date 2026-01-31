//! Bytecode compiler scaffolding.
#![allow(dead_code)]

use crate::bytecode::{BytecodeFunction, BytecodeModule, Instruction, OpCode};
use crate::helpers::{is_simple_string_literal, number_to_value};
use crate::types::JSValue;
use crate::api::parse_numeric_expr;
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
        let value = compile_literal(ctx, src)?;
        let mut func = BytecodeFunction::new(None);
        func.constants.push(value);
        func.code.push(Instruction { op: OpCode::Const, a: 0, b: 0, c: 0 });
        func.code.push(Instruction { op: OpCode::Return, a: 0, b: 0, c: 0 });
        Ok(BytecodeModule::new(func))
    }

    pub fn compile_empty(&mut self) -> BytecodeModule {
        BytecodeModule::new(BytecodeFunction::new(None))
    }
}

fn compile_literal(ctx: &mut JSContextImpl, src: &str) -> Result<JSValue, CompileError> {
    let s = src.trim();
    if is_simple_string_literal(s) {
        let inner = &s[1..s.len() - 1];
        return Ok(crate::api::js_new_string(ctx, inner));
    }
    if let Ok(num) = parse_numeric_expr(s) {
        return Ok(number_to_value(ctx, num));
    }
    Err(CompileError {
        message: "unsupported bytecode literal".to_string(),
    })
}
