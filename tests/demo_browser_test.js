#!/usr/bin/env node
import { Builder, logging } from "selenium-webdriver";
import chrome from "selenium-webdriver/chrome.js";

async function run() {
  const port = Number(process.env.DEMO_TEST_PORT || 8000);
  const url = `http://localhost:${port}/index.html?test`;

  console.log(`Testing GBX UI demo at ${url}`);

  const options = new chrome.Options();
  options.addArguments("--headless=new");
  options.addArguments("--disable-gpu");
  options.addArguments("--no-sandbox");
  options.addArguments("--disable-dev-shm-usage");

  const prefs = new logging.Preferences();
  prefs.setLevel(logging.Type.BROWSER, logging.Level.ALL);

  const driver = await new Builder()
    .forBrowser("chrome")
    .setChromeOptions(options)
    .setLoggingPrefs(prefs)
    .build();

  try {
    await driver.get(url);

    const seen = new Set();
    const collected = [];
    const deadline = Date.now() + 12_000;
    let done = false;

    while (!done && Date.now() < deadline) {
      const logs = await driver.manage().logs().get("browser");
      for (const entry of logs) {
        if (seen.has(entry.message)) {
          continue;
        }
        seen.add(entry.message);
        collected.push(entry.message);
        if (entry.message.includes("WASM_TEST_FAIL")) {
          console.log(entry.message);
          throw new Error(`Browser reported failure: ${entry.message}`);
        }
        if (entry.message.includes("WASM_TEST_DONE")) {
          done = true;
        }
      }
      if (!done) {
        await driver.sleep(250);
      }
    }

    if (!done) {
      throw new Error("Timed out waiting for WASM_TEST_DONE marker");
    }

    if (!collected.some((m) => m.includes("WASM_TEST_PASS:"))) {
      throw new Error("No WASM_TEST_PASS messages observed");
    }

    console.log("\n=== Browser Console Logs ===");
    collected.forEach((msg) => console.log(msg));
    console.log("===========================\n");

    console.log("✅ GBX UI integration test passed");
  } finally {
    await driver.quit();
  }
}

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
