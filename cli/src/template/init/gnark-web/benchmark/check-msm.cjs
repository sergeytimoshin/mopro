// Run from a generated web directory after building the diagnostic variant
// into web/msm-check and serving it with cross-origin isolation headers.
const { createRequire } = require("node:module");
const path = require("node:path");
const localRequire = createRequire(path.resolve("package.json"));
const { Builder } = localRequire("selenium-webdriver");
const chrome = localRequire("selenium-webdriver/chrome");
(async () => {
    const options = new chrome.Options().addArguments("--headless", "--no-sandbox");
    if (process.env.CHROME_BIN) options.setChromeBinaryPath(process.env.CHROME_BIN);
    const builder = new Builder().forBrowser("chrome").setChromeOptions(options);
    if (process.env.CHROMEDRIVER_BIN) builder.setChromeService(new chrome.ServiceBuilder(process.env.CHROMEDRIVER_BIN));
    const driver = await builder.build();
    try {
        await driver.manage().setTimeouts({ script: 120000 });
        for (const threads of [1, 16]) {
            await driver.get(process.env.MOPRO_GNARK_BENCH_URL || "http://localhost:3000/gnark-benchmark.html");
            const result = await driver.executeAsyncScript(function (threads, modulePath, done) {
                (async () => {
                    if (!crossOriginIsolated) throw new Error("MSM check requires cross-origin isolation");
                    const kernel = await import(modulePath);
                    await kernel.default();
                    await kernel.initThreadPool(threads);
                    if (kernel.msm_backend() !== "mcl") throw new Error("Expected mcl diagnostic build");
                    const sizes = [3, 0, 1, 31, 1025, 4097];
                    for (const n of sizes) kernel.check_msm(n);
                    return { threads, sizes };
                })().then(done, error => done({ error: String(error) }));
            }, threads, process.env.MOPRO_GNARK_MSM_CHECK_MODULE || "./msm-check/gnark_kernel.js");
            if (result.error) throw new Error(result.error);
            console.log(JSON.stringify(result));
        }
    } finally { await driver.quit(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
