//! Bytecode representation scaffolding.
#![allow(dead_code)]
//!
//! This is a placeholder layout to mirror MQuickJS bytecode concepts.

use crate::types::JSValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpCode {
    Nop,
    Const,
    LoadGlobal,
    StoreGlobal,
    Add,
    Sub,
    Mul,
    Div,
    Return,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub op: OpCode,
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

#[derive(Clone, Debug)]
pub struct BytecodeFunction {
    pub name: Option<String>,
    pub code: Vec<Instruction>,
    pub constants: Vec<JSValue>,
    pub stack_size: u32,
}

impl BytecodeFunction {
    pub fn new(name: Option<String>) -> Self {
        Self {
            name,
            code: Vec::new(),
            constants: Vec::new(),
            stack_size: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BytecodeModule {
    pub functions: Vec<BytecodeFunction>,
    pub main: usize,
}

impl BytecodeModule {
    pub fn new(main: BytecodeFunction) -> Self {
        Self {
            functions: vec![main],
            main: 0,
        }
    }
}
