// Trunk initializer: hook into WASM initialization to start the rayon thread pool.
// See: https://trunkrs.dev/assets/#initializer
export default function initRayon() {
    return {
        onStart() {},
        onProgress(_current, _total) {},
        onComplete() {},
        onSuccess(wasm) {
            // `wasm` is the `* as bindings` import — includes initThreadPool.
            // Workers only receive pre-decoded images (~26MB each), so we
            // can safely use all available cores.
            const cores = navigator.hardwareConcurrency || 4;
            console.log(`[rayon] Initializing thread pool with ${cores} workers…`);
            return wasm.initThreadPool(cores).then(() => {
                console.log('[rayon] Thread pool ready ✓');
            });
        },
        onFailure(error) {
            console.error('[rayon] WASM init failed:', error);
        },
    };
}
