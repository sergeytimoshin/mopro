// Run from MoproWasmBindings; no npm dependencies are needed.
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

// Optional destination installs the actual tarball for browser testing.
const destination = process.argv[2] && path.resolve(process.argv[2]);
const temporary = destination && fs.mkdtempSync(path.join(os.tmpdir(), "mopro-package-"));

const report = JSON.parse(execFileSync("npm", ["pack", "--json", ...(temporary ? ["--pack-destination", temporary] : ["--dry-run"])], {
    encoding: "utf8",
}));
const packages = Array.isArray(report) ? report : Object.values(report);
assert.equal(packages.length, 1);
const files = packages[0].files.map((file) => file.path);
for (const path of [
    "mopro_wasm_lib.js",
    "mopro_wasm_lib.d.ts",
    "gnark/gnark.js",
    "gnark/gnark.d.ts",
    "gnark/gnark.worker.js",
    "gnark/gnark.wasm",
    "gnark/wasm_exec.js",
    "gnark/LICENSE-APACHE",
    "gnark/accelerator/gnark_kernel.js",
    "gnark/accelerator/gnark_kernel_bg.wasm",
    "gnark/accelerator/LICENSE-APACHE",
    "gnark/accelerator/LICENSE-MIT",
]) {
    assert(files.includes(path), `npm package omits ${path}`);
}
assert(files.some((path) => path.startsWith("gnark/accelerator/snippets/") &&
    path.endsWith("/workerHelpers.no-bundler.js")), "npm package omits Rayon thread helpers");

assert(files.some((path) => path.startsWith("snippets/") &&
    path.endsWith("/workerHelpers.no-bundler.js")), "npm package omits top-level Rayon helpers");
console.log("Gnark npm package contains both runtimes and thread helpers.");
if (temporary) {
    try {
        const archive = path.join(temporary, packages[0].filename);
        execFileSync("tar", ["-xzf", archive, "-C", temporary]);
        fs.rmSync(destination, { recursive: true, force: true });
        fs.renameSync(path.join(temporary, "package"), destination);
        console.log(`Installed npm tarball into ${destination}`);
    } finally { fs.rmSync(temporary, { recursive: true, force: true }); }
}
