use muon_js::{
    JSCStringBuf, JS_EVAL_RETVAL, JS_Eval, JS_GetException, JS_GetPropertyStr,
    JS_NewContext, JS_SetLogFunc, JS_ToCString, JS_ToString, JSValue,
};
use std::fs;
use std::io::Write;

const VERSION: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/MUONJS_VERSION"));

fn log_func(_opaque: *mut core::ffi::c_void, data: *const u8, len: usize) {
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    let _ = std::io::stdout().write_all(bytes);
}

fn print_usage() {
    eprintln!("muonjs {} — MuonJS JavaScript runtime", VERSION.trim());
    eprintln!();
    eprintln!("Usage: muonjs [options] [file.js]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -e <code>     Evaluate JavaScript code");
    eprintln!("  --version     Print version and exit");
    eprintln!("  --help        Print this help and exit");
}

fn eval_and_print(source: &str, filename: &str) -> i32 {
    let mut mem = vec![0u8; 32 * 1024 * 1024];
    let mut ctx = JS_NewContext(&mut mem);
    JS_SetLogFunc(&mut ctx, Some(log_func));

    let val = JS_Eval(&mut ctx, source, filename, JS_EVAL_RETVAL);

    if val.is_exception() {
        let exc = JS_GetException(&mut ctx);
        let exc_str = JS_ToString(&mut ctx, exc);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let msg = JS_ToCString(&mut ctx, exc_str, &mut buf);
        if !msg.is_empty() {
            eprintln!("Exception: {}", msg);
        }
        let name_val = JS_GetPropertyStr(&mut ctx, exc, "name");
        let message_val = JS_GetPropertyStr(&mut ctx, exc, "message");
        let name_string = {
            let s = JS_ToString(&mut ctx, name_val);
            JS_ToCString(&mut ctx, s, &mut buf).to_string()
        };
        let message_string = {
            let s = JS_ToString(&mut ctx, message_val);
            JS_ToCString(&mut ctx, s, &mut buf).to_string()
        };
        if !name_string.is_empty() || !message_string.is_empty() {
            eprintln!("{}: {}", name_string, message_string);
        }
        let stack = JS_GetPropertyStr(&mut ctx, exc, "stack");
        if !stack.is_undefined() && !stack.is_null() {
            let stack_str = JS_ToString(&mut ctx, stack);
            let stack_msg = JS_ToCString(&mut ctx, stack_str, &mut buf);
            if !stack_msg.is_empty() {
                eprintln!("{}", stack_msg);
            }
        }
        return 1;
    }

    // Print result for -e expressions
    if val.is_bool() {
        println!("{}", val == JSValue::TRUE);
    } else if val.is_null() {
        println!("null");
    } else if val.is_undefined() {
        // Don't print undefined for file execution
    } else if let Some(n) = val.int32() {
        println!("{}", n);
    } else if let Some(bytes) = ctx.string_bytes(val) {
        if let Ok(s) = std::str::from_utf8(bytes) {
            println!("{}", s);
        }
    } else if let Ok(n) = muon_js::js_to_number(&mut ctx, val) {
        println!("{}", n);
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-V" => {
                println!("muonjs {}", VERSION.trim());
                return;
            }
            "--help" | "-h" => {
                print_usage();
                return;
            }
            "-e" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("muonjs: -e requires an argument");
                    std::process::exit(1);
                }
                let code = &args[i];
                std::process::exit(eval_and_print(code, "<eval>"));
            }
            arg if arg.starts_with('-') => {
                eprintln!("muonjs: unknown option: {}", arg);
                std::process::exit(1);
            }
            _ => {
                // File argument
                let filename = &args[i];
                let source = match fs::read_to_string(filename) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("muonjs: cannot read '{}': {}", filename, e);
                        std::process::exit(1);
                    }
                };
                std::process::exit(eval_and_print(&source, filename));
            }
        }
    }
}
