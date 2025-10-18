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
    await driver.sleep(5000);

    const logs = await driver.manage().logs().get("browser");
    const messages = logs.map((entry) => entry.message);

    console.log("\n=== Browser Console Logs ===");
    messages.forEach((msg) => console.log(msg));
    console.log("===========================\n");

    const failures = messages.filter((m) => m.includes("WASM_TEST_FAIL"));
    if (failures.length > 0) {
      throw new Error(`Browser reported failures: ${failures.join("\n")}`);
    }

    const passed = messages.filter((m) => m.includes("WASM_TEST_PASS:"));
    if (passed.length === 0) {
      throw new Error("No WASM_TEST_PASS messages observed");
    }

    if (!messages.some((m) => m.includes("WASM_TEST_DONE"))) {
      throw new Error("Missing WASM_TEST_DONE marker");
    }

    console.log("✅ wasm transport browser smoke tests passed");
  } finally {
    await driver.quit();
  }
}

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
