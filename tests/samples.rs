use muon_js::{JS_EVAL_RETVAL, JS_Eval, JS_NewContext, JS_Parse, JS_Run, JSValue};

const SAMPLE_FILES: &[(&str, &str)] = &[
    ("01_basic_expr.js", include_str!("../samples/01_basic_expr.js")),
    ("02_vars.js", include_str!("../samples/02_vars.js")),
    ("03_arrays.js", include_str!("../samples/03_arrays.js")),
    ("04_objects.js", include_str!("../samples/04_objects.js")),
    ("05_strings.js", include_str!("../samples/05_strings.js")),
];

fn eval_sample(source: &str, filename: &str) -> JSValue {
    let mut mem = vec![0u8; 4096];
    let mut ctx = JS_NewContext(&mut mem);
    JS_Eval(&mut ctx, source, filename, JS_EVAL_RETVAL)
}

fn parse_and_run_sample(source: &str, filename: &str) -> JSValue {
    let mut mem = vec![0u8; 4096];
    let mut ctx = JS_NewContext(&mut mem);
    let parsed = JS_Parse(&mut ctx, source, filename, JS_EVAL_RETVAL);
    if parsed.is_exception() {
        return parsed;
    }
    JS_Run(&mut ctx, parsed)
}

#[test]
fn eval_samples() {
    for (name, src) in SAMPLE_FILES {
        let val = eval_sample(src, name);
        assert!(!val.is_exception(), "eval failed for {name}");
    }
}

#[test]
fn parse_and_run_samples() {
    for (name, src) in SAMPLE_FILES {
        let val = parse_and_run_sample(src, name);
        assert!(!val.is_exception(), "parse/run failed for {name}");
    }
}
