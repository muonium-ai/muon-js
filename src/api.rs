use crate::context::Context;
use crate::value::Value;

/// Opaque handle to a VM instance.
pub type JSContext = Context;

/// Word-sized value tagged to match MQuickJS layout.
pub type JSValue = Value;

/// Create a new context with a caller-provided memory buffer.
/// This mirrors JS_NewContext in mquickjs.h and must stay API-compatible.
pub fn js_new_context(mem: &mut [u8]) -> JSContext {
    Context::new(mem)
}

/// Free the context. Finalizers should run; no system allocator is used.
pub fn js_free_context(_ctx: JSContext) {
    // Placeholder until GC/finalizers are implemented.
}
