//! Bytecode compiler scaffolding.
#![allow(dead_code)]

use crate::bytecode::{BytecodeFunction, BytecodeModule};

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
}

pub struct Compiler;

impl Compiler {
    pub fn new() -> Self {
        Self
    }

    pub fn compile_program(&mut self, _src: &str) -> Result<BytecodeModule, CompileError> {
        Err(CompileError {
            message: "bytecode compiler not implemented".to_string(),
        })
    }

    pub fn compile_empty(&mut self) -> BytecodeModule {
        BytecodeModule::new(BytecodeFunction::new(None))
    }
}
