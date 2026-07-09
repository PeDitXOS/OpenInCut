/**
 * Pruebas visuales globales: abre la app (dev server), ejecuta un guion de
 * interacciones y guarda screenshots numeradas en screenshots/<fecha>/.
 * Uso: node scripts/screenshot.mjs [url]
 */
import { spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "playwright";

const url = process.argv[2] ?? "http://localhost:5175";

// arrancar vite si no está corriendo (harness autosuficiente)
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
  console.log("dev server no responde: arrancando vite…");
  devServer = spawn("npm", ["run", "dev"], { stdio: "ignore" });
  for (let i = 0; i < 40 && !(await reachable()); i++) {
    await new Promise((r) => setTimeout(r, 500));
  }
  if (!(await reachable())) {
    devServer.kill("SIGKILL");
    throw new Error("no se pudo arrancar el dev server");
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
const effectSelect = page.locator("select").filter({ hasText: "Añadir efecto" });
await effectSelect.selectOption("core.chroma_key");
await page.waitForTimeout(200);
const chromaVisible = await page.getByText("Chroma Key").count();
if (chromaVisible === 0) throw new Error("el efecto Chroma Key no apareció en el Inspector");
await effectSelect.selectOption("core.color_correct");
await shot("07-panel-de-efectos");

// 8. Panel de Texto: palabras de la transcripción, marcar dos para cortar
await page.getByRole("button", { name: /Texto/ }).click();
await page.waitForTimeout(250);
await page.getByText("eee", { exact: false }).first().click();
await page.getByText("bueno", { exact: false }).first().click();
const cutBtn = await page.getByText(/Cortar 2 palabra/).count();
if (cutBtn === 0) throw new Error("la selección de palabras no se refleja en el botón de corte");
await shot("08-edicion-por-texto");

// 9. Modo vertical: genera la secuencia 1080x1920 y verifica el cambio
await page.getByRole("button", { name: /Medios/ }).click();
await page.getByTitle(/Genera una copia vertical/).click();
await page.waitForTimeout(300);
const vertRes = await page.getByText("1080×1920").count();
if (vertRes === 0) throw new Error("la secuencia vertical no está activa (no se ve 1080×1920)");
await shot("09-modo-vertical");

// 10. Rango I-O + diálogo de export: marcar rango con I/O y abrir el diálogo
await page.getByTitle(/secuencia activa/i).selectOption({ index: 0 }).catch(() => {});
await page.locator("header").click();
await page.keyboard.press("Home");
await page.keyboard.press("i");
for (let k = 0; k < 5; k++) await page.keyboard.press("Shift+ArrowRight");
await page.keyboard.press("o");
await page.getByRole("button", { name: /Exportar/ }).click();
await page.waitForTimeout(200);
if ((await page.getByText("YouTube 1080p").count()) === 0)
  throw new Error("el diálogo de export no muestra los presets");
const rangeRadio = page.locator("label", { hasText: "Rango I–O" }).locator('input[type="radio"]');
if (await rangeRadio.isDisabled())
  throw new Error("el rango I–O marcado con teclas no llegó al diálogo (radio deshabilitado)");
await rangeRadio.check();
await shot("10-dialogo-export");
await page.getByRole("button", { name: "Cancelar" }).click();

// 11. Selección por marco: arrastrar un rectángulo sobre varios clips
await page.keyboard.press("Shift+x"); // limpiar rango I-O para no ensuciar la regla
const tlBox = await canvas.boundingBox();
await page.mouse.move(tlBox.x + 470, tlBox.y + tlBox.height - 8);
await page.mouse.down();
await page.mouse.move(tlBox.x + 160, tlBox.y + 40, { steps: 8 });
await shot("11-marquee-seleccion");
await page.mouse.up();
const multiSel = await page.getByText(/clips seleccionados/).count();
if (multiSel === 0)
  throw new Error("la selección por marco no seleccionó varios clips (StatusBar)");

// 12. Keyframes: añadir un key de Posición X en el playhead desde el Inspector
await page.mouse.click(xForSec(4), box.y + 26 + 2 + 54 + 26); // seleccionar clip V1
await page.waitForTimeout(200);
const addKey = page.getByTitle(/Añadir keyframe en el playhead/).first();
await addKey.click();
await page.waitForTimeout(200);
if ((await page.getByTitle(/Quitar el keyframe del playhead/).count()) === 0)
  throw new Error("el keyframe no se creó (no hay ◆ en el playhead)");
// el editor de curvas aparece para la propiedad animada
const curveCanvas = page.getByLabel("Editor de curvas").first();
if ((await curveCanvas.count()) === 0)
  throw new Error("el editor de curvas no apareció en el Inspector");
// doble click en un punto vacío añade un segundo key (y el pie muestra ayuda)
const cbox = await curveCanvas.boundingBox();
await page.mouse.dblclick(cbox.x + cbox.width * 0.75, cbox.y + cbox.height * 0.3);
await page.waitForTimeout(250);
await shot("12-keyframe-creado");

// 13. Generadores: añadir un rectángulo y editar su color en el Inspector
await page.locator("header").click();
await page.keyboard.press("Home");
await page.getByRole("button", { name: /Forma/ }).click();
await page.waitForTimeout(250);
await page.mouse.click(xForSec(0.5), box.y + 26 + 2 + 27); // clip generador en V2
await page.waitForTimeout(250);
if ((await page.getByText("Generador", { exact: true }).count()) === 0)
  throw new Error("el Inspector no muestra el panel Generador");
const tipoSelect = page.locator("select").filter({ hasText: "Rectángulo sólido" });
if ((await tipoSelect.count()) === 0)
  throw new Error("el selector de tipo de generador no está");
await shot("13-generador-rectangulo");

await browser.close();
console.log(`\nScreenshots en ${outDir}`);
