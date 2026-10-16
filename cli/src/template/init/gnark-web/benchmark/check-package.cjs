// Run from MoproWasmBindings; no npm dependencies are needed.
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const accelerator = process.env.MOPRO_GNARK_ACCELERATOR || "false";
assert(["false", "true"].includes(accelerator), "MOPRO_GNARK_ACCELERATOR must be false or true");

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
    "gnark/gnark.backend.js",
    "gnark/gnark.wasm",
    "gnark/wasm_exec.js",
    "gnark/LICENSE-APACHE",
]) {
    assert(files.includes(path), `npm package omits ${path}`);
}
if (accelerator === "true") {
    for (const file of ["gnark_kernel.js", "gnark_kernel_bg.wasm", "LICENSE-APACHE", "LICENSE-MIT"]) {
        assert(files.includes(`gnark/accelerator/${file}`), `npm package omits accelerator/${file}`);
    }
    assert(files.some((path) => path.startsWith("gnark/accelerator/snippets/") &&
        path.endsWith("/workerHelpers.no-bundler.js")), "npm package omits Rayon thread helpers");
} else {
    assert(!files.some((path) => path.startsWith("gnark/accelerator/")), "Go-only package includes accelerator artifacts");
    for (const file of files.filter(path => path.startsWith("gnark/") && path.endsWith(".js"))) {
        assert.doesNotMatch(fs.readFileSync(file, "utf8"), /gnark_kernel|["']\.\/accelerator\//,
            `Go-only package references the accelerator in ${file}`);
    }
}
assert(!files.some(path => path.startsWith("gnark/backends/")), "Package includes unselected backend sources");

assert(files.some((path) => path.startsWith("snippets/") &&
    path.endsWith("/workerHelpers.no-bundler.js")), "npm package omits top-level Rayon helpers");
console.log(`Gnark npm package matches accelerator=${accelerator}.`);
if (temporary) {
    try {
        const archive = path.join(temporary, packages[0].filename);
        execFileSync("tar", ["-xzf", archive, "-C", temporary]);
        fs.rmSync(destination, { recursive: true, force: true });
        fs.renameSync(path.join(temporary, "package"), destination);
        console.log(`Installed npm tarball into ${destination}`);
    } finally { fs.rmSync(temporary, { recursive: true, force: true }); }
}
