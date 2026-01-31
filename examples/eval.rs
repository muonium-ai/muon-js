use muon_js::{JSCStringBuf, JS_EVAL_RETVAL, JS_Eval, JS_GetException, JS_NewContext, JS_ToCString, JS_ToString};
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.js>", args[0]);
        std::process::exit(1);
    }
    
    let filename = &args[1];
    let source = fs::read_to_string(filename).expect("Failed to read file");
    
    let mut mem = vec![0u8; 65536];
    let mut ctx = JS_NewContext(&mut mem);
    
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
