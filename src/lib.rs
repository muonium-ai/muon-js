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
}
