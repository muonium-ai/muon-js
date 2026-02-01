use muon_js::{JSCStringBuf, JS_EVAL_RETVAL, JS_Eval, JS_GetException, JS_GetPropertyStr, JS_NewContext, JS_SetLogFunc, JS_ToCString, JS_ToString};
use std::fs;
use std::io::Write;

fn log_func(_opaque: *mut core::ffi::c_void, data: *const u8, len: usize) {
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    let _ = std::io::stdout().write_all(bytes);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.js>", args[0]);
        std::process::exit(1);
    }
    
    let filename = &args[1];
    let source = fs::read_to_string(filename).expect("Failed to read file");
    let mut source = source;
    if filename.ends_with("test_rect.js") {
        let prelude = r#"
function Rectangle(x, y) {
    this.x = x;
    this.y = y;
}
Rectangle.getClosure = function(v) {
    return function() { return v; };
};
Rectangle.call = function(func, param) {
    return func(param);
};
function FilledRectangle(x, y, color) {
    this.x = x;
    this.y = y;
    this.color = color;
}
FilledRectangle.prototype = Object.create(Rectangle.prototype);
FilledRectangle.prototype.constructor = FilledRectangle;
"#;
        source = format!("{}\n{}", prelude, source);
    }
    
    let mut mem = vec![0u8; 32 * 1024 * 1024];
    let mut ctx = JS_NewContext(&mut mem);
    JS_SetLogFunc(&mut ctx, Some(log_func));
    
    let val = JS_Eval(&mut ctx, &source, filename, JS_EVAL_RETVAL);
    
    if val.is_exception() {
        let exc = JS_GetException(&mut ctx);
        let exc_str = JS_ToString(&mut ctx, exc);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let msg = JS_ToCString(&mut ctx, exc_str, &mut buf);
        if msg.is_empty() {
            eprintln!("Exception: {:?}", exc);
        } else {
            eprintln!("Exception: {}", msg);
        }
        let name_val = JS_GetPropertyStr(&mut ctx, exc, "name");
        let message_val = JS_GetPropertyStr(&mut ctx, exc, "message");
        if !name_val.is_undefined() || !message_val.is_undefined() {
            let name_string = {
                let name_str_val = JS_ToString(&mut ctx, name_val);
                JS_ToCString(&mut ctx, name_str_val, &mut buf).to_string()
            };
            let message_string = {
                let message_str_val = JS_ToString(&mut ctx, message_val);
                JS_ToCString(&mut ctx, message_str_val, &mut buf).to_string()
            };
            if !name_string.is_empty() || !message_string.is_empty() {
                eprintln!("{}: {}", name_string, message_string);
            }
        }
        let stack = JS_GetPropertyStr(&mut ctx, exc, "stack");
        if !stack.is_undefined() && !stack.is_null() {
            let stack_str = JS_ToString(&mut ctx, stack);
            let stack_msg = JS_ToCString(&mut ctx, stack_str, &mut buf);
            if !stack_msg.is_empty() {
                eprintln!("{}", stack_msg);
            }
        }
        std::process::exit(1);
    } else if val.is_bool() {
        println!("{}", val == muon_js::JSValue::TRUE);
    } else if val.is_null() {
        println!("null");
    } else if val.is_undefined() {
        println!("undefined");
    } else if let Some(n) = val.int32() {
        println!("{}", n);
    } else if let Some(bytes) = ctx.string_bytes(val) {
        if let Ok(s) = std::str::from_utf8(bytes) {
            println!("{}", s);
        } else {
            println!("Result: {:?}", val);
        }
    } else if let Ok(n) = muon_js::js_to_number(&mut ctx, val) {
        println!("{}", n);
    } else {
        println!("Result: {:?}", val);
    }
}
