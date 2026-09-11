import * as gnark from './MoproWasmBindings/gnark/gnark.js';

// The browser driver calls the public API from this production bundle.
globalThis.gnarkBundlerTest = gnark;
