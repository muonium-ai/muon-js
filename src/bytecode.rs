//! Bytecode representation scaffolding.
#![allow(dead_code)]
//!
//! This is a placeholder layout to mirror MQuickJS bytecode concepts.

use crate::types::JSValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpCode {
    Nop,
    Const,         // a = constant pool index
    LoadGlobal,    // a = constant pool index (name string)
    StoreGlobal,   // a = constant pool index (name string)
    LoadLocal,     // a = local slot index
    StoreLocal,    // a = local slot index
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    StrictEq,
    StrictNeq,
    Lt,
    Gt,
    Le,
    Ge,
    Not,
    And,
    Or,
    Drop,
    Dup,
    Jump,          // a = target pc
    JumpIfFalse,   // a = target pc
    JumpIfTrue,    // a = target pc
    Return,
    Call,          // a = argc (function + args on stack)
    GetProp,       // pop key, pop obj, push obj[key]
    SetProp,       // pop value, pop key, pop obj, set obj[key]=value, push value
    GetElem,       // pop index, pop obj, push obj[index]  
    Concat,        // pop b, pop a, push string(a) + string(b)
    ToNumber,      // pop value, push Number(value)
    Typeof,        // pop value, push typeof value
    Neg,           // pop value, push -value
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    BitNot,
    IncLocal,      // a = local slot, b = amount (as i32 encoded in u32)
    Pop,           // alias for Drop semantically - pop and discard
    CallMethod,    // a = argc; stack: [... this_obj, func, arg0..argN-1] → [result]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub op: OpCode,
    pub a: u32,
    pub b: u32,
}

#[derive(Clone, Debug)]
pub struct BytecodeFunction {
    pub name: Option<String>,
    pub code: Vec<Instruction>,
    pub constants: Vec<JSValue>,
    pub string_constants: Vec<String>,
    pub locals: Vec<String>,
    pub stack_size: u32,
}

impl BytecodeFunction {
    pub fn new(name: Option<String>) -> Self {
        Self {
            name,
            code: Vec::new(),
            constants: Vec::new(),
            string_constants: Vec::new(),
            locals: Vec::new(),
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
