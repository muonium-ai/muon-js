//! Bytecode VM — executes compiled bytecode modules.
#![allow(dead_code)]

use crate::bytecode::{BytecodeModule, OpCode};
use crate::types::{JSObjectClassEnum, JSValue};
use crate::{api::{js_throw_error, js_to_number, js_to_string, js_value_to_atom,
                   js_get_property_str, js_get_property_uint32, js_new_string, js_new_string_len,
                   js_is_string, js_is_function, js_is_number, int_to_decimal_bytes}, JSContextImpl};
use crate::value::Value;

pub struct VM;

macro_rules! pop {
    ($stack:expr, $ctx:expr) => {
        match $stack.pop() {
            Some(v) => v,
            None => return js_throw_error($ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow"),
        }
    };
}

impl VM {
    pub fn new() -> Self {
        Self
    }

    pub fn run_module(&mut self, ctx: &mut JSContextImpl, module: &BytecodeModule) -> JSValue {
        self.run_module_with_locals(ctx, module, &[])
    }

    pub fn run_module_with_locals(
        &mut self,
        ctx: &mut JSContextImpl,
        module: &BytecodeModule,
        param_values: &[JSValue],
    ) -> JSValue {
        if module.functions.is_empty() {
            return JSValue::UNDEFINED;
        }
        let func = &module.functions[module.main];
        let num_locals = func.locals.len();
        let mut locals: Vec<JSValue> = vec![JSValue::UNDEFINED; num_locals];
        // Fill in parameter values
        for (i, &v) in param_values.iter().enumerate() {
            if i < num_locals {
                locals[i] = v;
            }
        }
        let mut stack: Vec<JSValue> = Vec::with_capacity(32);
        let global = crate::api::js_get_global_object(ctx);
        let mut pc = 0usize;
        let code = &func.code;
        let constants = &func.constants;
        let code_len = code.len();

        while pc < code_len {
            let ins = code[pc];
            match ins.op {
                OpCode::Nop => {}

                OpCode::Const => {
                    let idx = ins.a as usize;
                    if let Some(val) = constants.get(idx) {
                        stack.push(*val);
                    } else {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range");
                    }
                }

                OpCode::LoadLocal => {
                    let slot = ins.a as usize;
                    let val = if slot < locals.len() { locals[slot] } else { JSValue::UNDEFINED };
                    stack.push(val);
                }

                OpCode::StoreLocal => {
                    let slot = ins.a as usize;
                    let val = pop!(stack, ctx);
                    if slot < locals.len() {
                        locals[slot] = val;
                    }
                    stack.push(val);
                }

                OpCode::IncLocal => {
                    let slot = ins.a as usize;
                    let amount = ins.b as i32;
                    if slot < locals.len() {
                        let old = locals[slot];
                        if old.is_int() {
                            if let Some(n) = old.int32() {
                                locals[slot] = JSValue::from_int32(n.wrapping_add(amount));
                            }
                        } else {
                            let n = match js_to_number(ctx, old) {
                                Ok(n) => n,
                                Err(e) => return e,
                            };
                            locals[slot] = crate::helpers::number_to_value(ctx, n + amount as f64);
                        }
                    }
                }

                OpCode::LoadGlobal => {
                    let idx = ins.a as usize;
                    let name_val = match constants.get(idx) {
                        Some(v) => *v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range"),
                    };
                    let atom = js_value_to_atom(ctx, name_val);
                    if atom <= 0 {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name not string");
                    }
                    let mut val = ctx
                        .get_property_atom_id(global, atom as u32)
                        .unwrap_or(JSValue::UNDEFINED);
                    if val.is_undefined() {
                        // VM global loads bypass eval_value(), so unresolved globals
                        // need builtin marker fallback (Math, JSON, console, parseInt...).
                        if let Some(name_bytes) = ctx.string_bytes(name_val) {
                            let name_buf = name_bytes.to_vec();
                            if !ctx.has_property_str(global, &name_buf) {
                                if let Ok(name) = core::str::from_utf8(&name_buf) {
                                    if let Some(resolved) = crate::evals::eval_value(ctx, name) {
                                        val = resolved;
                                    }
                                }
                            }
                        }
                    }
                    stack.push(val);
                }

                OpCode::StoreGlobal => {
                    let idx = ins.a as usize;
                    let name_val = match constants.get(idx) {
                        Some(v) => *v,
                        None => return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode const out of range"),
                    };
                    let atom = js_value_to_atom(ctx, name_val);
                    if atom <= 0 {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode global name not string");
                    }
                    let value = pop!(stack, ctx);
                    if !ctx.set_property_atom_id(global, atom as u32, value) {
                        return js_throw_error(ctx, JSObjectClassEnum::TypeError, "property set failed");
                    }
                    stack.push(value);
                }

                OpCode::Add => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    // Fast path: int + int (avoids string_bytes checks)
                    if a.is_int() && b.is_int() {
                        let ai = a.int32().unwrap_or(0);
                        let bi = b.int32().unwrap_or(0);
                        stack.push(crate::helpers::number_to_value(ctx, ai as f64 + bi as f64));
                    } else {
                        let a_is_str = ctx.string_bytes(a).is_some();
                        if a_is_str && b.is_int() {
                            // Fast path: string + int — avoid intermediate string alloc
                            let n = b.int32().unwrap_or(0);
                            let mut int_buf = [0u8; 12];
                            let int_bytes = int_to_decimal_bytes(n, &mut int_buf);
                            if let Some(header) = ctx.alloc_string_concat_val(a, int_bytes) {
                                stack.push(Value::from_ptr(header));
                            } else {
                                return js_throw_error(ctx, JSObjectClassEnum::InternalError, "out of memory");
                            }
                        } else if a_is_str || ctx.string_bytes(b).is_some() {
                            let ls = js_to_string(ctx, a);
                            let rs = js_to_string(ctx, b);
                            let lb = ctx.string_bytes(ls).unwrap_or(b"");
                            let rb = ctx.string_bytes(rs).unwrap_or(b"");
                            let mut out = Vec::with_capacity(lb.len() + rb.len());
                            out.extend_from_slice(lb);
                            out.extend_from_slice(rb);
                            stack.push(js_new_string_len(ctx, &out));
                        } else {
                            let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                            let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                            stack.push(crate::helpers::number_to_value(ctx, an + bn));
                        }
                    }
                }

                OpCode::Sub => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if a.is_int() && b.is_int() {
                        let ai = a.int32().unwrap_or(0);
                        let bi = b.int32().unwrap_or(0);
                        stack.push(crate::helpers::number_to_value(ctx, ai as f64 - bi as f64));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(crate::helpers::number_to_value(ctx, an - bn));
                    }
                }

                OpCode::Mul => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if a.is_int() && b.is_int() {
                        let ai = a.int32().unwrap_or(0) as i64;
                        let bi = b.int32().unwrap_or(0) as i64;
                        let result = ai * bi;
                        if result >= i32::MIN as i64 && result <= i32::MAX as i64 {
                            stack.push(JSValue::from_int32(result as i32));
                        } else {
                            stack.push(crate::helpers::number_to_value(ctx, result as f64));
                        }
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(crate::helpers::number_to_value(ctx, an * bn));
                    }
                }

                OpCode::Div => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, an / bn));
                }

                OpCode::Mod => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, an % bn));
                }

                OpCode::Neg => {
                    let v = pop!(stack, ctx);
                    let n = match js_to_number(ctx, v) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, -n));
                }

                OpCode::Eq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_loose_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(result));
                }

                OpCode::Neq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_loose_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(!result));
                }

                OpCode::StrictEq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_strict_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(result));
                }

                OpCode::StrictNeq => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let result = vm_strict_eq(ctx, a, b);
                    stack.push(JSValue::new_bool(!result));
                }

                OpCode::Lt => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if a.is_int() && b.is_int() {
                        stack.push(JSValue::new_bool(a.int32().unwrap_or(0) < b.int32().unwrap_or(0)));
                    } else if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la < lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an < bn));
                    }
                }

                OpCode::Gt => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if a.is_int() && b.is_int() {
                        stack.push(JSValue::new_bool(a.int32().unwrap_or(0) > b.int32().unwrap_or(0)));
                    } else if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la > lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an > bn));
                    }
                }

                OpCode::Le => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if a.is_int() && b.is_int() {
                        stack.push(JSValue::new_bool(a.int32().unwrap_or(0) <= b.int32().unwrap_or(0)));
                    } else if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la <= lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an <= bn));
                    }
                }

                OpCode::Ge => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    if a.is_int() && b.is_int() {
                        stack.push(JSValue::new_bool(a.int32().unwrap_or(0) >= b.int32().unwrap_or(0)));
                    } else if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
                        stack.push(JSValue::new_bool(la >= lb));
                    } else {
                        let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                        let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                        stack.push(JSValue::new_bool(an >= bn));
                    }
                }

                OpCode::Not => {
                    let v = pop!(stack, ctx);
                    let truthy = crate::evals::is_truthy(ctx, v);
                    stack.push(JSValue::new_bool(!truthy));
                }

                OpCode::And | OpCode::Or => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let a_truthy = crate::evals::is_truthy(ctx, a);
                    let result = if ins.op == OpCode::And {
                        if a_truthy { b } else { a }
                    } else if a_truthy {
                        a
                    } else {
                        b
                    };
                    stack.push(result);
                }

                OpCode::Drop | OpCode::Pop => {
                    let _ = stack.pop();
                }

                OpCode::Dup => {
                    let v = pop!(stack, ctx);
                    stack.push(v);
                    stack.push(v);
                }

                OpCode::Jump => {
                    pc = ins.a as usize;
                    continue;
                }

                OpCode::JumpIfFalse => {
                    let v = pop!(stack, ctx);
                    if !crate::evals::is_truthy(ctx, v) {
                        pc = ins.a as usize;
                        continue;
                    }
                }

                OpCode::JumpIfTrue => {
                    let v = pop!(stack, ctx);
                    if crate::evals::is_truthy(ctx, v) {
                        pc = ins.a as usize;
                        continue;
                    }
                }

                OpCode::Call => {
                    let argc = ins.a as usize;
                    // Stack: [... func, arg0, arg1, ..., argN-1]
                    if stack.len() < argc + 1 {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow in call");
                    }
                    let args_start = stack.len() - argc;
                    let func_idx = args_start - 1;
                    let func_val = stack[func_idx];
                    // Fast path: direct C function dispatch
                    if let Some((cf_idx, cf_params)) = ctx.c_function_info(func_val) {
                        let result = if argc <= 8 {
                            let mut args_buf = [JSValue::UNDEFINED; 8];
                            args_buf[..argc].copy_from_slice(&stack[args_start..]);
                            stack.truncate(func_idx);
                            crate::api::call_c_function_direct(ctx, cf_idx, cf_params, global, &args_buf[..argc])
                        } else {
                            let args: Vec<JSValue> = stack[args_start..].to_vec();
                            stack.truncate(func_idx);
                            crate::api::call_c_function_direct(ctx, cf_idx, cf_params, global, &args)
                        };
                        stack.push(result);
                    } else if argc == 1 {
                        // Inline fast path for common single-arg builtins: Number(v)
                        let arg0 = stack[args_start];
                        let mut handled = false;
                        if let Some(marker_bytes) = ctx.string_bytes(func_val) {
                            if marker_bytes == b"__builtin_Number__" {
                                stack.truncate(func_idx);
                                if arg0.is_int() {
                                    stack.push(arg0);
                                } else {
                                    let n = match js_to_number(ctx, arg0) { Ok(n) => n, Err(e) => return e };
                                    stack.push(crate::helpers::number_to_value(ctx, n));
                                }
                                handled = true;
                            }
                        }
                        if !handled {
                            let mut args_buf = [JSValue::UNDEFINED; 8];
                            args_buf[0] = arg0;
                            stack.truncate(func_idx);
                            let result = crate::api::call_function_value(ctx, func_val, global, &args_buf[..1]);
                            if let Some(result) = result {
                                stack.push(result);
                            } else {
                                return js_throw_error(ctx, JSObjectClassEnum::TypeError, "not a function");
                            }
                        }
                    } else {
                        // Use stack buffer for small arg counts to avoid Vec heap allocation
                        let result = if argc <= 8 {
                            let mut args_buf = [JSValue::UNDEFINED; 8];
                            args_buf[..argc].copy_from_slice(&stack[args_start..]);
                            stack.truncate(func_idx);
                            crate::api::call_function_value(ctx, func_val, global, &args_buf[..argc])
                        } else {
                            let args: Vec<JSValue> = stack[args_start..].to_vec();
                            stack.truncate(func_idx);
                            crate::api::call_function_value(ctx, func_val, global, &args)
                        };
                        if let Some(result) = result {
                            stack.push(result);
                        } else {
                            return js_throw_error(ctx, JSObjectClassEnum::TypeError, "not a function");
                        }
                    }
                }

                OpCode::CallMethod => {
                    let argc = ins.a as usize;
                    // Stack: [... this_obj, func, arg0, arg1, ..., argN-1]
                    if stack.len() < argc + 2 {
                        return js_throw_error(ctx, JSObjectClassEnum::InternalError, "bytecode stack underflow in call_method");
                    }
                    let args_start = stack.len() - argc;
                    let func_idx = args_start - 1;
                    let this_idx = func_idx - 1;
                    let func_val = stack[func_idx];
                    let this_val = stack[this_idx];
                    // Fast path: direct C function dispatch (skips call_function_value overhead)
                    if let Some((cf_idx, cf_params)) = ctx.c_function_info(func_val) {
                        let result = if argc <= 8 {
                            let mut args_buf = [JSValue::UNDEFINED; 8];
                            args_buf[..argc].copy_from_slice(&stack[args_start..]);
                            stack.truncate(this_idx);
                            crate::api::call_c_function_direct(ctx, cf_idx, cf_params, this_val, &args_buf[..argc])
                        } else {
                            let args: Vec<JSValue> = stack[args_start..].to_vec();
                            stack.truncate(this_idx);
                            crate::api::call_c_function_direct(ctx, cf_idx, cf_params, this_val, &args)
                        };
                        stack.push(result);
                    } else {
                        let result = if argc <= 8 {
                            let mut args_buf = [JSValue::UNDEFINED; 8];
                            args_buf[..argc].copy_from_slice(&stack[args_start..]);
                            stack.truncate(this_idx);
                            crate::api::call_function_value(ctx, func_val, this_val, &args_buf[..argc])
                        } else {
                            let args: Vec<JSValue> = stack[args_start..].to_vec();
                            stack.truncate(this_idx);
                            crate::api::call_function_value(ctx, func_val, this_val, &args)
                        };
                        if let Some(result) = result {
                            stack.push(result);
                        } else {
                            return js_throw_error(ctx, JSObjectClassEnum::TypeError, "not a function");
                        }
                    }
                }

                OpCode::GetProp => {
                    let key = pop!(stack, ctx);
                    let obj = pop!(stack, ctx);
                    if let Some(_key_bytes) = ctx.string_bytes(key) {
                        // Copy key bytes to release ctx borrow. Stack buffer for short keys,
                        // heap Vec for keys > 128 bytes.
                        let key_buf = copy_string_bytes(ctx, key);
                        // Check if obj is a builtin marker (e.g. __builtin_console__)
                        let is_marker = ctx.string_bytes(obj)
                            .map(|b| b.len() >= 13 && b.starts_with(b"__builtin_") && b.ends_with(b"__"))
                            .unwrap_or(false);
                        if is_marker {
                            let val = resolve_builtin_marker_prop(ctx, obj, &key_buf);
                            stack.push(val);
                        } else {
                            let key_str = core::str::from_utf8(&key_buf).unwrap_or("");
                            let val = js_get_property_str(ctx, obj, key_str);
                            // If property is undefined and obj is a string,
                            // resolve as a string method marker for CallMethod dispatch
                            if val.is_undefined() && ctx.string_bytes(obj).is_some() {
                                let marker = resolve_string_method_marker(ctx, &key_buf);
                                stack.push(marker);
                            } else {
                                stack.push(val);
                            }
                        }
                    } else if key.is_int() {
                        let idx = key.int32().unwrap_or(0) as u32;
                        let val = js_get_property_uint32(ctx, obj, idx);
                        stack.push(val);
                    } else {
                        let key_s = js_to_string(ctx, key);
                        let key_buf = copy_string_bytes(ctx, key_s);
                        let key_str = core::str::from_utf8(&key_buf).unwrap_or("");
                        let val = js_get_property_str(ctx, obj, key_str);
                        stack.push(val);
                    }
                }

                OpCode::SetProp => {
                    let value = pop!(stack, ctx);
                    let key = pop!(stack, ctx);
                    let obj = pop!(stack, ctx);
                    if let Some(_key_bytes) = ctx.string_bytes(key) {
                        let key_buf = copy_string_bytes(ctx, key);
                        let key_str = core::str::from_utf8(&key_buf).unwrap_or("");
                        crate::api::js_set_property_str(ctx, obj, key_str, value);
                    }
                    stack.push(value);
                }

                OpCode::GetElem => {
                    let idx = pop!(stack, ctx);
                    let obj = pop!(stack, ctx);
                    if idx.is_int() {
                        let i = idx.int32().unwrap_or(0) as u32;
                        let val = js_get_property_uint32(ctx, obj, i);
                        stack.push(val);
                    } else if let Some(_idx_bytes) = ctx.string_bytes(idx) {
                        let idx_buf = copy_string_bytes(ctx, idx);
                        let idx_str = core::str::from_utf8(&idx_buf).unwrap_or("");
                        let val = js_get_property_str(ctx, obj, idx_str);
                        stack.push(val);
                    } else {
                        let n = match js_to_number(ctx, idx) { Ok(n) => n, Err(e) => return e };
                        let i = n as u32;
                        let val = js_get_property_uint32(ctx, obj, i);
                        stack.push(val);
                    }
                }

                OpCode::Concat => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    // Fast path: string + int — avoid intermediate string alloc
                    if ctx.string_bytes(a).is_some() && b.is_int() {
                        let n = b.int32().unwrap_or(0);
                        let mut int_buf = [0u8; 12];
                        let int_bytes = int_to_decimal_bytes(n, &mut int_buf);
                        if let Some(header) = ctx.alloc_string_concat_val(a, int_bytes) {
                            stack.push(Value::from_ptr(header));
                        } else {
                            return js_throw_error(ctx, JSObjectClassEnum::InternalError, "out of memory");
                        }
                    } else {
                        let ls = js_to_string(ctx, a);
                        let rs = js_to_string(ctx, b);
                        let lb = ctx.string_bytes(ls).unwrap_or(b"");
                        let rb = ctx.string_bytes(rs).unwrap_or(b"");
                        let mut out = Vec::with_capacity(lb.len() + rb.len());
                        out.extend_from_slice(lb);
                        out.extend_from_slice(rb);
                        stack.push(js_new_string_len(ctx, &out));
                    }
                }

                OpCode::ToNumber => {
                    let v = pop!(stack, ctx);
                    let n = match js_to_number(ctx, v) { Ok(n) => n, Err(e) => return e };
                    stack.push(crate::helpers::number_to_value(ctx, n));
                }

                OpCode::Typeof => {
                    let v = pop!(stack, ctx);
                    let type_str = if v.is_bool() {
                        "boolean"
                    } else if js_is_number(ctx, v) != 0 {
                        "number"
                    } else if js_is_string(ctx, v) != 0 {
                        "string"
                    } else if v.is_undefined() {
                        "undefined"
                    } else if v.is_null() {
                        "object"
                    } else if js_is_function(ctx, v) != 0 {
                        "function"
                    } else if v.is_ptr() {
                        "object"
                    } else {
                        "undefined"
                    };
                    stack.push(js_new_string(ctx, type_str));
                }

                OpCode::BitAnd => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) & (bn as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::BitOr => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) | (bn as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::BitXor => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) ^ (bn as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::Shl => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) << ((bn as u32) & 31);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::Shr => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = (an as i32) >> ((bn as u32) & 31);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::UShr => {
                    let b = pop!(stack, ctx);
                    let a = pop!(stack, ctx);
                    let an = match js_to_number(ctx, a) { Ok(n) => n, Err(e) => return e };
                    let bn = match js_to_number(ctx, b) { Ok(n) => n, Err(e) => return e };
                    let result = ((an as u32) >> ((bn as u32) & 31)) as i32;
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::BitNot => {
                    let v = pop!(stack, ctx);
                    let n = match js_to_number(ctx, v) { Ok(n) => n, Err(e) => return e };
                    let result = !(n as i32);
                    stack.push(JSValue::from_int32(result));
                }

                OpCode::Return => {
                    return stack.pop().unwrap_or(JSValue::UNDEFINED);
                }
            }
            pc += 1;
        }
        JSValue::UNDEFINED
    }
}

/// Copy string bytes from a JSValue into an owned buffer, releasing the ctx borrow.
/// Uses a stack-allocated array for keys up to 128 bytes (covers nearly all real-world
/// property names), and falls back to a heap Vec for longer keys.
enum KeyBuf {
    Stack([u8; 128], usize),
    Heap(Vec<u8>),
}

impl core::ops::Deref for KeyBuf {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        match self {
            KeyBuf::Stack(buf, len) => &buf[..*len],
            KeyBuf::Heap(vec) => vec.as_slice(),
        }
    }
}

fn copy_string_bytes(ctx: &mut JSContextImpl, val: JSValue) -> KeyBuf {
    if let Some(bytes) = ctx.string_bytes(val) {
        let len = bytes.len();
        if len <= 128 {
            let mut buf = [0u8; 128];
            buf[..len].copy_from_slice(bytes);
            KeyBuf::Stack(buf, len)
        } else {
            KeyBuf::Heap(bytes.to_vec())
        }
    } else {
        KeyBuf::Stack([0u8; 128], 0)
    }
}

/// Loose equality (==) for the VM.
fn vm_loose_eq(ctx: &mut JSContextImpl, a: JSValue, b: JSValue) -> bool {
    if (a.is_null() && b.is_undefined()) || (a.is_undefined() && b.is_null()) {
        return true;
    }
    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
        return la == lb;
    }
    if a.0 == b.0 {
        return true;
    }
    let ln = js_to_number(ctx, a).ok();
    let rn = js_to_number(ctx, b).ok();
    if let (Some(l), Some(r)) = (ln, rn) {
        l == r
    } else {
        false
    }
}

/// Strict equality (===) for the VM.
/// Per ES spec: different types → false, no type coercion.
fn vm_strict_eq(ctx: &mut JSContextImpl, a: JSValue, b: JSValue) -> bool {
    // Identical bit patterns are always equal (covers ints, bools, null, undefined, same-ptr objects)
    if a.0 == b.0 {
        return true;
    }
    // String comparison by content (two different pointers can hold the same string)
    if let (Some(la), Some(lb)) = (ctx.string_bytes(a), ctx.string_bytes(b)) {
        return la == lb;
    }
    // Different types → false. Check type tags to prevent cross-type numeric coercion.
    // A bool must not equal a number, null must not equal undefined, etc.
    if js_type_tag(ctx, a) != js_type_tag(ctx, b) {
        return false;
    }
    // Same type, different bit pattern — only numbers (int vs float) need further check
    let ln = js_to_number(ctx, a).ok();
    let rn = js_to_number(ctx, b).ok();
    if let (Some(l), Some(r)) = (ln, rn) {
        l == r
    } else {
        false
    }
}

/// Return a type discriminant for strict equality type-checking.
/// Values: 0=undefined, 1=null, 2=bool, 3=number, 4=string, 5=object/function
fn js_type_tag(ctx: &mut JSContextImpl, v: JSValue) -> u8 {
    if v.is_undefined() {
        0
    } else if v.is_null() {
        1
    } else if v.is_bool() {
        2
    } else if v.is_int() || ctx.float_value(v).is_some() {
        3
    } else if ctx.string_bytes(v).is_some() {
        4
    } else {
        5
    }
}

/// Resolve a property access on a builtin marker string.
/// e.g. __builtin_console__ + "log" → __builtin_console_log__
/// Handles special numeric constants like Math.PI, Math.E.
fn resolve_builtin_marker_prop(ctx: &mut JSContextImpl, obj: JSValue, prop: &[u8]) -> JSValue {
    // Copy marker base to stack buffer to release ctx borrow
    let mut base_buf = [0u8; 32];
    let base_len = {
        let obj_bytes = ctx.string_bytes(obj).unwrap_or(b"");
        let blen = if obj_bytes.len() >= 12 { obj_bytes.len() - 12 } else { 0 };
        if blen > 0 && blen <= 32 {
            base_buf[..blen].copy_from_slice(&obj_bytes[10..obj_bytes.len() - 2]);
        }
        blen
    };
    // Handle Math numeric constants (return actual values, not markers)
    if base_len == 4 && &base_buf[..4] == b"Math" {
        match prop {
            b"PI" => return crate::helpers::number_to_value(ctx, core::f64::consts::PI),
            b"E" => return crate::helpers::number_to_value(ctx, core::f64::consts::E),
            b"LN2" => return crate::helpers::number_to_value(ctx, core::f64::consts::LN_2),
            b"LN10" => return crate::helpers::number_to_value(ctx, core::f64::consts::LN_10),
            b"LOG2E" => return crate::helpers::number_to_value(ctx, core::f64::consts::LOG2_E),
            b"LOG10E" => return crate::helpers::number_to_value(ctx, core::f64::consts::LOG10_E),
            b"SQRT2" => return crate::helpers::number_to_value(ctx, core::f64::consts::SQRT_2),
            _ => {}
        }
    }
    // General case: construct __builtin_{base}_{prop}__ marker
    let needed = 10 + base_len + 1 + prop.len() + 2;
    let mut buf = [0u8; 80];
    if needed <= buf.len() && base_len > 0 {
        buf[..10].copy_from_slice(b"__builtin_");
        buf[10..10 + base_len].copy_from_slice(&base_buf[..base_len]);
        buf[10 + base_len] = b'_';
        buf[11 + base_len..11 + base_len + prop.len()].copy_from_slice(prop);
        buf[11 + base_len + prop.len()..needed].copy_from_slice(b"__");
        js_new_string_len(ctx, &buf[..needed])
    } else {
        JSValue::UNDEFINED
    }
}

/// Construct a __builtin_string_{method}__ marker for string method dispatch.
fn resolve_string_method_marker(ctx: &mut JSContextImpl, method: &[u8]) -> JSValue {
    let prefix = b"__builtin_string_";
    let suffix = b"__";
    let needed = prefix.len() + method.len() + suffix.len();
    let mut buf = [0u8; 80];
    if needed <= buf.len() {
        buf[..prefix.len()].copy_from_slice(prefix);
        buf[prefix.len()..prefix.len() + method.len()].copy_from_slice(method);
        buf[prefix.len() + method.len()..needed].copy_from_slice(suffix);
        js_new_string_len(ctx, &buf[..needed])
    } else {
        JSValue::UNDEFINED
    }
}
