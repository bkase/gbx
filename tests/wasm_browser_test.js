#!/usr/bin/env node
import { Builder, logging } from "selenium-webdriver";
import chrome from "selenium-webdriver/chrome.js";

async function run() {
  const port = Number(process.env.WASM_TEST_PORT || 4510);
  const url = `http://localhost:${port}/index.html`;

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
    const deadline = Date.now() + 10_000;
    let done = false;
    let messages = [];

    while (!done && Date.now() < deadline) {
      const logs = await driver.manage().logs().get("browser");
      for (const entry of logs) {
        if (seen.has(entry.message)) {
          continue;
        }
        seen.add(entry.message);
        messages.push(entry.message);
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

    const passes = messages.filter((m) => m.includes("WASM_TEST_PASS:"));
    if (passes.length === 0) {
      throw new Error("No WASM_TEST_PASS messages observed");
    }

    console.log("\n=== Browser Console Logs ===");
    messages.forEach((msg) => console.log(msg));
    console.log("===========================\n");

    console.log("✅ wasm transport browser smoke tests passed");
  } finally {
    await driver.quit();
  }
}

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
