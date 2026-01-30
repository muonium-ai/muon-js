use muon_js::{JS_EVAL_RETVAL, JS_Eval, JS_NewContext, JS_Parse, JS_Run, JSValue};

const BASIC_SAMPLES: &[(&str, &str)] = &[
    ("01_arithmetic.js", include_str!("../samples/pass/01_arithmetic.js")),
    ("02_variables.js", include_str!("../samples/pass/02_variables.js")),
    ("03_strings.js", include_str!("../samples/pass/03_strings.js")),
    ("04_null_undefined.js", include_str!("../samples/pass/04_null_undefined.js")),
    ("05_arrays_simple.js", include_str!("../samples/pass/05_arrays_simple.js")),
    ("06_objects.js", include_str!("../samples/pass/06_objects.js")),
    ("07_array_indexing.js", include_str!("../samples/pass/07_array_indexing.js")),
    ("08_property_access.js", include_str!("../samples/pass/08_property_access.js")),
    ("09_functions.js", include_str!("../samples/pass/09_functions.js")),
    ("10_comparison.js", include_str!("../samples/pass/10_comparison.js")),
    ("11_if_else.js", include_str!("../samples/pass/11_if_else.js")),
    ("12_for_loop.js", include_str!("../samples/pass/12_for_loop.js")),
    ("13_while_loop.js", include_str!("../samples/pass/13_while_loop.js")),
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
    for (name, src) in BASIC_SAMPLES {
        let val = eval_sample(src, name);
        assert!(!val.is_exception(), "eval failed for {name}");
    }
}

#[test]
fn parse_and_run_samples() {
    for (name, src) in BASIC_SAMPLES {
        let val = parse_and_run_sample(src, name);
        assert!(!val.is_exception(), "parse/run failed for {name}");
    }
}
