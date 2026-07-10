/**
 * Global visual checks: opens the app (dev server), runs a script of
 * interactions and saves numbered screenshots in screenshots/<date>/.
 * Usage: node scripts/screenshot.mjs [url]
 */
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";

const url = process.argv[2] ?? "http://localhost:5175";

// start vite if it is not running (self-sufficient harness)
let devServer = null;
const reachable = async () => {
  try {
    const r = await fetch(url, { signal: AbortSignal.timeout(1500) });
    return r.ok;
  } catch {
    return false;
  }
};
if (!(await reachable())) {
  console.log("dev server not responding: starting vite…");
  devServer = spawn("npm", ["run", "dev"], { stdio: "ignore" });
  for (let i = 0; i < 40 && !(await reachable()); i++) {
    await new Promise((r) => setTimeout(r, 500));
  }
  if (!(await reachable())) {
    devServer.kill("SIGKILL");
    throw new Error("could not start the dev server");
  }
}
const cleanup = () => devServer?.kill("SIGKILL");
process.on("exit", cleanup);
process.on("uncaughtException", (e) => {
  cleanup();
  throw e;
});
const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, "-");
const outDir = join("screenshots", stamp);
mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch();
const page = await browser.newPage({
  viewport: { width: 1600, height: 950 },
  deviceScaleFactor: 2,
});

const shot = async (name) => {
  await page.waitForTimeout(350);
  await page.screenshot({ path: join(outDir, `${name}.png`) });
  console.log(`✓ ${name}.png`);
};

await page.goto(url, { waitUntil: "networkidle" });
await page.waitForSelector("#timeline-canvas");
// wait for fonts and two render frames (avoids races with the first paint
// and with Vite's dependency-optimization reload in dev)
await page.evaluate(() => document.fonts.ready);
await page.evaluate(
  () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))),
);
await page.waitForTimeout(400);

// 1. Initial state
await shot("01-initial-shell");

// 2. Select a video clip in the timeline (click on V1, ~t=4s)
const canvas = page.locator("#timeline-canvas");
const box = await canvas.boundingBox();
// geometry: ruler 26 + V2(52+2) + V1 above → y of V1's center ≈ 26+2+54+26
const xForSec = (sec) => box.x + sec * 26; // initial pxPerSec = 26, viewStart = 0
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26);
await shot("02-clip-selected-inspector");

// 3. Split the clip at the playhead (S key) — the demo playhead is at 12.4s
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26); // make sure it is selected
await page.keyboard.press("Home");
await page.mouse.click(xForSec(6), box.y + 10); // seek on the ruler to ~6s
await page.keyboard.press("s");
await shot("03-split-at-playhead");

// 4. Undo and check visually
await page.keyboard.press(process.platform === "darwin" ? "Meta+z" : "Control+z");
await shot("04-after-undo");

// 5. Centered zoom in (ctrl+wheel simulated via the zoom slider)
await page.locator('input[title="Zoom"]').fill("80");
await shot("05-high-zoom");

// 6. Play ~1.2 s (drop the slider focus first: shortcuts ignore inputs)
await page.locator('input[title="Zoom"]').fill("26");
await page.locator("header").click();
const tcBefore = await page
  .locator('div[title="Current position"]')
  .textContent();
await page.keyboard.press("Space");
await page.waitForTimeout(1200);
await page.keyboard.press("Space");
const tcAfter = await page
  .locator('div[title="Current position"]')
  .textContent();
if (tcBefore === tcAfter)
  throw new Error(`playback did not advance the timecode (${tcBefore})`);
console.log(`  playback: ${tcBefore} → ${tcAfter}`);
await shot("06-after-playing");

// 7. Modular effects: select a clip and add chroma key from the Inspector
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26);
await page.waitForTimeout(200);
const effectSelect = page.locator("select").filter({ hasText: "Add effect" });
await effectSelect.selectOption("core.chroma_key");
await page.waitForTimeout(200);
const chromaVisible = await page.getByText("Chroma Key").count();
if (chromaVisible === 0) throw new Error("the Chroma Key effect did not appear in the Inspector");
await effectSelect.selectOption("core.color_correct");
await shot("07-effects-panel");

// 8. Text panel: transcript words, mark two of them to cut
await page.getByRole("button", { name: /Text/ }).click();
await page.waitForTimeout(250);
// scoped to the left panel and exact: "physics" also appears in the project title
const textPanel = page.locator("aside").first();
await textPanel.getByText("physics", { exact: true }).click();
await textPanel.getByText("collisions", { exact: true }).click();
const cutBtn = await page.getByText(/Cut 2 word/).count();
if (cutBtn === 0) throw new Error("the word selection is not reflected in the cut button");
await shot("08-text-based-editing");

// 9. Vertical mode: generate the 1080x1920 sequence and verify the change
await page.getByRole("button", { name: /Media/ }).click();
await page.getByTitle(/Generate a vertical copy/).click();
await page.waitForTimeout(300);
const vertRes = await page.getByText("1080×1920").count();
if (vertRes === 0) throw new Error("the vertical sequence is not active (1080×1920 not visible)");
await shot("09-vertical-mode");

// 10. I-O range + export dialog: mark the range with I/O and open the dialog
await page.getByTitle(/active sequence/i).selectOption({ index: 0 }).catch(() => {});
await page.locator("header").click();
await page.keyboard.press("Home");
await page.keyboard.press("i");
for (let k = 0; k < 5; k++) await page.keyboard.press("Shift+ArrowRight");
await page.keyboard.press("o");
await page.getByRole("button", { name: /Export/ }).click();
await page.waitForTimeout(200);
if ((await page.getByText("YouTube 1080p").count()) === 0)
  throw new Error("the export dialog does not show the presets");
const rangeRadio = page.locator("label", { hasText: "Range I–O" }).locator('input[type="radio"]');
if (await rangeRadio.isDisabled())
  throw new Error("the I–O range marked with keys did not reach the dialog (radio disabled)");
await rangeRadio.check();
await shot("10-export-dialog");
await page.getByRole("button", { name: "Cancel" }).click();

// 11. Marquee selection: drag a rectangle over several clips
await page.keyboard.press("Shift+x"); // clear the I-O range so it does not clutter the ruler
const tlBox = await canvas.boundingBox();
await page.mouse.move(tlBox.x + 470, tlBox.y + tlBox.height - 8);
await page.mouse.down();
await page.mouse.move(tlBox.x + 160, tlBox.y + 40, { steps: 8 });
await shot("11-marquee-selection");
await page.mouse.up();
const multiSel = await page.getByText(/clips selected/).count();
if (multiSel === 0)
  throw new Error("the marquee selection did not select several clips (StatusBar)");

// 12. Keyframes: add a Position X key at the playhead from the Inspector
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26); // select the V1 clip
await page.waitForTimeout(200);
const addKey = page.getByTitle(/Add keyframe at the playhead/).first();
await addKey.click();
await page.waitForTimeout(200);
if ((await page.getByTitle(/Remove the keyframe at the playhead/).count()) === 0)
  throw new Error("the keyframe was not created (no ◆ at the playhead)");
// the curve editor appears for the animated property
const curveCanvas = page.getByLabel("Curve editor").first();
if ((await curveCanvas.count()) === 0)
  throw new Error("the curve editor did not appear in the Inspector");
// double click on an empty point adds a second key (and the footer shows help)
const cbox = await curveCanvas.boundingBox();
await page.mouse.dblclick(cbox.x + cbox.width * 0.75, cbox.y + cbox.height * 0.3);
await page.waitForTimeout(250);
await shot("12-keyframe-created");

// 13. Generators: add a rectangle and edit its color in the Inspector
await page.locator("header").click();
await page.keyboard.press("Home");
await page.getByRole("button", { name: /Shape/ }).click();
await page.waitForTimeout(250);
await page.mouse.click(xForSec(0.5), box.y + 26 + 2 + 27); // generator clip on V2
await page.waitForTimeout(250);
if ((await page.getByText("Generator", { exact: true }).count()) === 0)
  throw new Error("the Inspector does not show the Generator panel");
const kindSelect = page.locator("select").filter({ hasText: "Solid rectangle" });
if ((await kindSelect.count()) === 0)
  throw new Error("the generator type selector is missing");
await shot("13-generator-rectangle");

await browser.close();
console.log(`\nScreenshots in ${outDir}`);
