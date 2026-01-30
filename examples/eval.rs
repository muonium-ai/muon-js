use muon_js::{JS_EVAL_RETVAL, JS_Eval, JS_NewContext};
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <file.js>", args[0]);
        std::process::exit(1);
    }
    
    let filename = &args[1];
    let source = fs::read_to_string(filename).expect("Failed to read file");
    
    let mut mem = vec![0u8; 8192];
    let mut ctx = JS_NewContext(&mut mem);
    
    let val = JS_Eval(&mut ctx, &source, filename, JS_EVAL_RETVAL);
    
    if val.is_exception() {
        println!("Exception");
    } else if let Some(n) = val.int32() {
        println!("{}", n);
    } else if val.is_bool() {
        println!("{}", val == muon_js::JSValue::TRUE);
    } else if val.is_null() {
        println!("null");
    } else if val.is_undefined() {
        println!("undefined");
    } else if let Some(bytes) = ctx.string_bytes(val) {
        if let Ok(s) = std::str::from_utf8(bytes) {
            println!("{}", s);
        } else {
            println!("Result: {:?}", val);
        }
    } else {
        println!("Result: {:?}", val);
    }
}
