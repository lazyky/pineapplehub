// Trunk initializer: hook into WASM initialization to start the rayon thread pool.
// Also drives the loading screen progress bar.
// See: https://trunkrs.dev/assets/#initializer

function setProgress(percent, status) {
    const bar = document.getElementById('loading-bar');
    const text = document.getElementById('loading-status');
    if (bar) bar.style.width = `${Math.min(percent, 100)}%`;
    if (text) text.textContent = status;
}

function hideLoadingScreen() {
    const screen = document.getElementById('loading-screen');
    if (!screen) return;
    screen.classList.add('fade-out');
    setTimeout(() => {
        screen.remove();
        console.log('[loading] Screen removed');
    }, 700);
}

export default function initRayon() {
    return {
        onStart() {
            setProgress(2, 'Downloading WASM…');
        },
        onProgress(current, total) {
            if (total > 0) {
                const pct = Math.round((current / total) * 80);
                const mb = (current / 1048576).toFixed(1);
                const totalMb = (total / 1048576).toFixed(1);
                setProgress(pct, `Downloading… ${mb} / ${totalMb} MB`);
            }
        },
        onComplete() {
            setProgress(82, 'Compiling WASM…');
        },
        onSuccess(wasm) {
            setProgress(90, 'Starting thread pool…');
            const cores = navigator.hardwareConcurrency || 4;
            console.log(`[rayon] Initializing thread pool with ${cores} workers…`);

            try {
                const result = wasm.initThreadPool(cores);

                // Handle both Promise and synchronous return
                if (result && typeof result.then === 'function') {
                    return result.then(() => {
                        setProgress(100, 'Ready!');
                        console.log('[rayon] Thread pool ready ✓');
                        setTimeout(hideLoadingScreen, 300);
                    });
                } else {
                    // Synchronous — pool is already ready
                    setProgress(100, 'Ready!');
                    console.log('[rayon] Thread pool ready ✓');
                    setTimeout(hideLoadingScreen, 300);
                }
            } catch (err) {
                console.error('[rayon] Thread pool error:', err);
                setProgress(100, 'Ready');
                setTimeout(hideLoadingScreen, 300);
            }
        },
        onFailure(error) {
            setProgress(0, `Failed: ${error}`);
            const bar = document.getElementById('loading-bar');
            if (bar) {
                bar.style.background = '#e74c3c';
                bar.style.boxShadow = '0 0 16px rgba(231,76,60,0.6)';
            }
            console.error('[rayon] WASM init failed:', error);
        },
    };
}
