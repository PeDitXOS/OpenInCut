import { chromium } from "playwright";

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1600, height: 950 },
  deviceScaleFactor: 2,
});
page.on("console", (msg) => console.log("[console]", msg.type(), msg.text()));
page.on("pageerror", (err) => console.log("[pageerror]", err.message));
await page.goto("http://localhost:5175", { waitUntil: "networkidle" });
await page.waitForTimeout(1000);
const info = await page.evaluate(() => {
  const canvases = [...document.querySelectorAll("canvas")];
  return canvases.map((c) => {
    const ctx = c.getContext("2d");
    const data = ctx?.getImageData(Math.floor(c.width / 2), Math.floor(c.height / 2), 1, 1).data;
    return {
      id: c.id || "(preview)",
      w: c.width,
      h: c.height,
      styleW: c.style.width,
      centerPixel: data ? [...data] : null,
    };
  });
});
console.log(JSON.stringify(info, null, 2));
await browser.close();
