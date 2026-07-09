/**
 * Pruebas visuales globales: abre la app (dev server), ejecuta un guion de
 * interacciones y guarda screenshots numeradas en screenshots/<fecha>/.
 * Uso: node scripts/screenshot.mjs [url]
 */
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";

const url = process.argv[2] ?? "http://localhost:5175";
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
// esperar fuentes y dos frames de render (evita carreras con la primera pintura
// y con la recarga de optimización de dependencias de Vite en dev)
await page.evaluate(() => document.fonts.ready);
await page.evaluate(
  () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))),
);
await page.waitForTimeout(400);

// 1. Estado inicial
await shot("01-shell-inicial");

// 2. Seleccionar un clip de video del timeline (click sobre V1, ~t=4s)
const canvas = page.locator("#timeline-canvas");
const box = await canvas.boundingBox();
// geometría: regla 26 + V2(52+2) + V1 arriba → y del centro de V1 ≈ 26+2+54+26
const xForSec = (sec) => box.x + sec * 26; // pxPerSec inicial = 26, viewStart = 0
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26);
await shot("02-clip-seleccionado-inspector");

// 3. Dividir el clip en el playhead (tecla S) — el playhead demo está en 12.4s
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26); // asegurar selección
await page.keyboard.press("Home");
await page.mouse.click(xForSec(6), box.y + 10); // seek en la regla a ~6s
await page.keyboard.press("s");
await shot("03-split-en-playhead");

// 4. Deshacer y comprobar visualmente
await page.keyboard.press(process.platform === "darwin" ? "Meta+z" : "Control+z");
await shot("04-despues-de-deshacer");

// 5. Zoom in centrado (ctrl+wheel simulada vía slider de zoom)
await page.locator('input[title="Zoom"]').fill("80");
await shot("05-zoom-alto");

// 6. Reproducir ~1.2 s (quitar el foco del slider primero: los atajos ignoran inputs)
await page.locator('input[title="Zoom"]').fill("26");
await page.locator("header").click();
const tcBefore = await page
  .locator('div[title="Posición actual"]')
  .textContent();
await page.keyboard.press("Space");
await page.waitForTimeout(1200);
await page.keyboard.press("Space");
const tcAfter = await page
  .locator('div[title="Posición actual"]')
  .textContent();
if (tcBefore === tcAfter)
  throw new Error(`la reproducción no avanzó el timecode (${tcBefore})`);
console.log(`  reproducción: ${tcBefore} → ${tcAfter}`);
await shot("06-tras-reproducir");

// 7. Efectos modulares: seleccionar clip y añadir chroma key desde el Inspector
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26);
await page.waitForTimeout(200);
const effectSelect = page.locator("select");
await effectSelect.selectOption("core.chroma_key");
await page.waitForTimeout(200);
const chromaVisible = await page.getByText("Chroma Key").count();
if (chromaVisible === 0) throw new Error("el efecto Chroma Key no apareció en el Inspector");
await effectSelect.selectOption("core.color_correct");
await shot("07-panel-de-efectos");

await browser.close();
console.log(`\nScreenshots en ${outDir}`);
