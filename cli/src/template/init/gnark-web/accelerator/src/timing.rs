#[cfg(all(target_arch = "wasm32", feature = "bench-profile"))]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    pub fn now() -> f64;
}
#[cfg(not(all(target_arch = "wasm32", feature = "bench-profile")))]
pub fn now() -> f64 {
    0.
}
pub fn run<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = now();
    let value = f();
    (value, now() - start)
}
