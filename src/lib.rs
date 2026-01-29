//! Muon JS: a native Rust port of MQuickJS (not a wrapper).

mod api;
mod context;
mod types;
mod value;

pub use api::*;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_no_holes() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let arr = JS_NewArray(&mut ctx, 0);
        assert!(!arr.is_exception());
        let v1 = JS_NewInt32(&mut ctx, 42);
        let r0 = JS_SetPropertyUint32(&mut ctx, arr, 0, v1);
        assert!(!r0.is_exception());
        let r2 = JS_SetPropertyUint32(&mut ctx, arr, 2, v1);
        assert!(r2.is_exception());
    }

    #[test]
    fn object_property_roundtrip() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let obj = JS_NewObject(&mut ctx);
        let v = JS_NewInt32(&mut ctx, 7);
        let r = JS_SetPropertyStr(&mut ctx, obj, "x", v);
        assert!(!r.is_exception());
        let got = JS_GetPropertyStr(&mut ctx, obj, "x");
        assert!(JS_IsNumber(&mut ctx, got) != 0);
        let val = JS_ToInt32(&mut ctx, got).expect("int32");
        assert_eq!(val, 7);
    }

    #[test]
    fn array_length_rules() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let arr = JS_NewArray(&mut ctx, 0);
        let zero = JS_NewInt32(&mut ctx, 0);
        let ok = JS_SetPropertyStr(&mut ctx, arr, "length", zero);
        assert!(!ok.is_exception());
        let one = JS_NewInt32(&mut ctx, 1);
        let bad = JS_SetPropertyStr(&mut ctx, arr, "length", one);
        assert!(bad.is_exception());
        let five = JS_NewInt32(&mut ctx, 5);
        let _ = JS_SetPropertyUint32(&mut ctx, arr, 0, five);
        let shrink = JS_SetPropertyStr(&mut ctx, arr, "length", zero);
        assert!(!shrink.is_exception());
    }

    #[test]
    fn to_string_primitives() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let t = JSValue::new_bool(true);
        let ts = JS_ToString(&mut ctx, t);
        let s = JS_ToCString(&mut ctx, ts, &mut buf);
        assert_eq!(s, "true");
        let u = JSValue::UNDEFINED;
        let us = JS_ToString(&mut ctx, u);
        let su = JS_ToCString(&mut ctx, us, &mut buf);
        assert_eq!(su, "undefined");
    }

    #[test]
    fn opaque_roundtrip() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let obj = JS_NewObject(&mut ctx);
        let ptr = 0x1234usize as *mut core::ffi::c_void;
        JS_SetOpaque(&mut ctx, obj, ptr);
        let got = JS_GetOpaque(&mut ctx, obj);
        assert_eq!(got, ptr);
    }

    #[test]
    fn bytecode_magic_check() {
        let magic = JS_BYTECODE_MAGIC.to_ne_bytes();
        let buf = [magic[0], magic[1], 0, 0];
        assert_eq!(JS_IsBytecode(&buf), 1);
        let bad = [0u8; 4];
        assert_eq!(JS_IsBytecode(&bad), 0);
    }

    #[test]
    fn global_object_property() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let global = JS_GetGlobalObject(&mut ctx);
        let v = JS_NewInt32(&mut ctx, 99);
        let r = JS_SetPropertyStr(&mut ctx, global, "g", v);
        assert!(!r.is_exception());
        let got = JS_GetPropertyStr(&mut ctx, global, "g");
        let val = JS_ToInt32(&mut ctx, got).expect("int32");
        assert_eq!(val, 99);
    }

    #[test]
    fn float_roundtrip() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let f = JS_NewFloat64(&mut ctx, 1.5);
        assert_eq!(JS_IsNumber(&mut ctx, f), 1);
        let n = JS_ToNumber(&mut ctx, f).expect("number");
        assert!((n - 1.5).abs() < 1e-9);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let fs = JS_ToString(&mut ctx, f);
        let s = JS_ToCString(&mut ctx, fs, &mut buf);
        assert!(s.starts_with('1'));
    }

    #[test]
    fn large_int_conversions() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let u = JS_NewUint32(&mut ctx, u32::MAX);
        let nu = JS_ToNumber(&mut ctx, u).expect("number");
        assert!((nu - 4_294_967_295.0).abs() < 1.0);
        let i = JS_NewInt64(&mut ctx, 1_i64 << 40);
        let ni = JS_ToNumber(&mut ctx, i).expect("number");
        assert!(ni > 1.0e12);
    }

    #[test]
    fn c_function_object() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let func = JS_NewCFunctionParams(&mut ctx, 1, JSValue::UNDEFINED);
        assert_eq!(JS_IsFunction(&mut ctx, func), 1);
    }

    #[test]
    fn eval_basic_literals() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let v = JS_Eval(&mut ctx, "42", "test.js", 0);
        let n = JS_ToInt32(&mut ctx, v).expect("int32");
        assert_eq!(n, 42);
        let t = JS_Eval(&mut ctx, "true", "test.js", 0);
        assert_eq!(t, JSValue::TRUE);
        let s = JS_Eval(&mut ctx, "\"hi\"", "test.js", 0);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let ss = JS_ToString(&mut ctx, s);
        let out = JS_ToCString(&mut ctx, ss, &mut buf);
        assert_eq!(out, "hi");
        let len = JS_GetPropertyStr(&mut ctx, s, "length");
        assert_eq!(JS_ToInt32(&mut ctx, len).unwrap(), 2);
        let e = JS_Eval(&mut ctx, "1+2*3", "test.js", 0);
        let n = JS_ToNumber(&mut ctx, e).expect("number");
        assert!((n - 7.0).abs() < 1e-9);
        let e2 = JS_Eval(&mut ctx, "(1+2)*3", "test.js", 0);
        let n2 = JS_ToNumber(&mut ctx, e2).expect("number");
        assert!((n2 - 9.0).abs() < 1e-9);
        let e3 = JS_Eval(&mut ctx, "1.5+1", "test.js", 0);
        let n3 = JS_ToNumber(&mut ctx, e3).expect("number");
        assert!((n3 - 2.5).abs() < 1e-9);
        let arr = JS_Eval(&mut ctx, "[1, 2]", "test.js", 0);
        let a1 = JS_GetPropertyUint32(&mut ctx, arr, 0);
        let a2 = JS_GetPropertyUint32(&mut ctx, arr, 1);
        assert_eq!(JS_ToInt32(&mut ctx, a1).unwrap(), 1);
        assert_eq!(JS_ToInt32(&mut ctx, a2).unwrap(), 2);
        let obj = JS_Eval(&mut ctx, "{a: 3}", "test.js", 0);
        let oa = JS_GetPropertyStr(&mut ctx, obj, "a");
        assert_eq!(JS_ToInt32(&mut ctx, oa).unwrap(), 3);
        let nested = JS_Eval(&mut ctx, "[1, [2, 3]]", "test.js", 0);
        let inner = JS_GetPropertyUint32(&mut ctx, nested, 1);
        let inner_val = JS_GetPropertyUint32(&mut ctx, inner, 0);
        assert_eq!(JS_ToInt32(&mut ctx, inner_val).unwrap(), 2);
    }

    #[test]
    fn parse_and_run() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let code = JS_Parse(&mut ctx, "42", "test.js", 0);
        let res = JS_Run(&mut ctx, code);
        let n = JS_ToInt32(&mut ctx, res).expect("int32");
        assert_eq!(n, 42);
    }

    #[test]
    fn throw_sets_exception() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let _ = JS_ThrowError(&mut ctx, JSObjectClassEnum::TypeError, "boom");
        let ex = JS_GetException(&mut ctx);
        let mut buf = JSCStringBuf { buf: [0u8; 5] };
        let es = JS_ToString(&mut ctx, ex);
        let s = JS_ToCString(&mut ctx, es, &mut buf);
        assert_eq!(s, "boom");
    }

    #[test]
    fn numeric_property_names_on_arrays() {
        let mut mem = vec![0u8; 4096];
        let mut ctx = JS_NewContext(&mut mem);
        let arr = JS_NewArray(&mut ctx, 0);
        let v = JS_NewInt32(&mut ctx, 3);
        let ok = JS_SetPropertyStr(&mut ctx, arr, "0", v);
        assert!(!ok.is_exception());
        let got = JS_GetPropertyUint32(&mut ctx, arr, 0);
        let n = JS_ToInt32(&mut ctx, got).expect("int32");
        assert_eq!(n, 3);
        let bad = JS_SetPropertyStr(&mut ctx, arr, "2", v);
        assert!(bad.is_exception());
    }
}
