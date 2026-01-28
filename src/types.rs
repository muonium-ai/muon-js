//! Public types and constants modeled after mquickjs.h.

use crate::value::Value;

pub type JSWord = usize;
pub type JSValue = Value;
pub type JSBool = i32;

pub const JS_EX_NORMAL: i32 = 0;
pub const JS_EX_CALL: i32 = 1;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JSObjectClassEnum {
    Object,
    Array,
    CFunction,
    Closure,
    Number,
    Boolean,
    String,
    Date,
    Regexp,

    Error,
    EvalError,
    RangeError,
    ReferenceError,
    SyntaxError,
    TypeError,
    UriError,
    InternalError,

    ArrayBuffer,
    TypedArray,

    Uint8cArray,
    Int8Array,
    Uint8Array,
    Int16Array,
    Uint16Array,
    Int32Array,
    Uint32Array,
    Float32Array,
    Float64Array,

    User,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JSCFunctionEnum {
    Bound,
    User,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JSCStringBuf {
    pub buf: [u8; 5],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JSGCRef {
    pub val: JSValue,
    pub prev: *mut JSGCRef,
}

pub type JSCFunction = fn(ctx: *mut JSContext, this_val: *mut JSValue, argc: i32, argv: *mut JSValue) -> JSValue;
pub type JSCFinalizer = fn(ctx: *mut JSContext, opaque: *mut core::ffi::c_void);

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JSCFunctionDefEnum {
    Generic,
    GenericMagic,
    Constructor,
    ConstructorMagic,
    GenericParams,
    FF,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union JSCFunctionType {
    pub generic: Option<JSCFunction>,
    pub generic_magic: Option<fn(*mut JSContext, *mut JSValue, i32, *mut JSValue, i32) -> JSValue>,
    pub constructor: Option<JSCFunction>,
    pub constructor_magic: Option<fn(*mut JSContext, *mut JSValue, i32, *mut JSValue, i32) -> JSValue>,
    pub generic_params: Option<fn(*mut JSContext, *mut JSValue, i32, *mut JSValue, JSValue) -> JSValue>,
    pub f_f: Option<fn(f64) -> f64>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JSCFunctionDef {
    pub func: JSCFunctionType,
    pub name: JSValue,
    pub def_type: u8,
    pub arg_count: u8,
    pub magic: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JSSTDLibraryDef {
    pub stdlib_table: *const JSWord,
    pub c_function_table: *const JSCFunctionDef,
    pub c_finalizer_table: *const JSCFinalizer,
    pub stdlib_table_len: u32,
    pub stdlib_table_align: u32,
    pub sorted_atoms_offset: u32,
    pub global_object_offset: u32,
    pub class_count: u32,
}

pub type JSWriteFunc = fn(opaque: *mut core::ffi::c_void, buf: *const u8, buf_len: usize);
/// Return non-zero to interrupt.
pub type JSInterruptHandler = fn(ctx: *mut JSContext, opaque: *mut core::ffi::c_void) -> i32;

pub const JS_EVAL_RETVAL: i32 = 1 << 0;
pub const JS_EVAL_REPL: i32 = 1 << 1;
pub const JS_EVAL_STRIP_COL: i32 = 1 << 2;
pub const JS_EVAL_JSON: i32 = 1 << 3;
pub const JS_EVAL_REGEXP: i32 = 1 << 4;
pub const JS_EVAL_REGEXP_FLAGS_SHIFT: i32 = 8;

pub const JS_BYTECODE_MAGIC: u16 = 0xacfb;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JSBytecodeHeader {
    pub magic: u16,
    pub version: u16,
    pub base_addr: usize,
    pub unique_strings: JSValue,
    pub main_func: JSValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JSBytecodeHeader32 {
    pub magic: u16,
    pub version: u16,
    pub base_addr: u32,
    pub unique_strings: u32,
    pub main_func: u32,
}

/// Opaque placeholder for the eventual runtime context.
#[repr(C)]
pub struct JSContext {
    _private: [u8; 0],
}
