/**
 * Real-engine proof of the webview preview compositor: loads a local video as a
 * native <video> (as the asset protocol would), plays it, draws it letterboxed
 * to a canvas exactly like drawMediaLayer, then draws a big image overlay
 * centred on top — and reads back pixels to prove the composite is correct.
 *
 * This is what actually failed before with ffmpeg-per-frame ("image over a
 * video stops playback"): here the video keeps its own clock and the overlay
 * never blocks it. Usage: node scripts/verify-compositor.mjs <mediaDir>
 */
import { chromium } from "playwright";
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { join, extname } from "node:path";

const mediaDir = process.argv[2];
if (!mediaDir) throw new Error("usage: verify-compositor.mjs <mediaDir>");

const TYPES = { ".mp4": "video/mp4", ".png": "image/png", ".jpg": "image/jpeg" };

// tiny static server WITH range support (video needs it, like Tauri's asset proto)
const server = createServer(async (req, res) => {
  try {
    const name = decodeURIComponent(req.url.split("?")[0]);
    const buf = await readFile(join(mediaDir, name));
    const type = TYPES[extname(name)] ?? "application/octet-stream";
    const cors = { "Access-Control-Allow-Origin": "*" };
    const range = req.headers.range;
    if (range) {
      const m = /bytes=(\d+)-(\d*)/.exec(range);
      const start = Number(m[1]);
      const end = m[2] ? Number(m[2]) : buf.length - 1;
      res.writeHead(206, {
        ...cors,
        "Content-Type": type,
        "Accept-Ranges": "bytes",
        "Content-Range": `bytes ${start}-${end}/${buf.length}`,
        "Content-Length": end - start + 1,
      });
      res.end(buf.subarray(start, end + 1));
    } else {
      res.writeHead(200, { ...cors, "Content-Type": type, "Accept-Ranges": "bytes", "Content-Length": buf.length });
      res.end(buf);
    }
  } catch (e) {
    res.writeHead(404);
    res.end(String(e));
  }
});
await new Promise((r) => server.listen(0, r));
const port = server.address().port;
const base = `http://localhost:${port}`;

const browser = await chromium.launch();
const page = await browser.newPage();
page.on("pageerror", (e) => console.log("[pageerror]", e.message));

const result = await page.evaluate(
  async ({ base }) => {
    // canvas represents a VERTICAL 1080x1920 sequence, fit into 405x720 on screen
    const cw = 1080, ch = 1920;
    const w = 405, h = 720;
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d");

    // ---- base video (native <video>, muted, plays on its own clock) ----
    const video = document.createElement("video");
    video.crossOrigin = "anonymous";
    video.muted = true;
    video.playsInline = true;
    video.src = `${base}/base_red.mp4`;
    await new Promise((res, rej) => {
      video.onloadeddata = res;
      video.onerror = () => rej(new Error("video failed to load"));
    });
    const videoReadyState = video.readyState;
    await video.play();
    // let it actually advance a few frames while we "hold an image over it"
    await new Promise((r) => setTimeout(r, 500));
    const advanced = video.currentTime > 0.1;

    // ---- overlay image ----
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.src = `${base}/overlay_green.png`;
    await new Promise((res, rej) => {
      img.onload = res;
      img.onerror = () => rej(new Error("image failed to load"));
    });

    // drawMediaLayer math (base fills, upper fits without upscaling), centred
    const drawLayer = (src, sw, sh, isBase) => {
      const k = w / cw;
      const fit = isBase
        ? Math.min(cw / sw, ch / sh)
        : Math.min(cw / sw, ch / sh, 1);
      const fw = sw * fit * k;
      const fh = sh * fit * k;
      ctx.drawImage(src, w / 2 - fw / 2, h / 2 - fh / 2, fw, fh);
    };

    // compose one frame: black, base video, image on top
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, w, h);
    drawLayer(video, video.videoWidth, video.videoHeight, true);
    // sample the base (centre of the letterboxed video band) BEFORE the overlay
    const topPix = [...ctx.getImageData(w / 2, h / 2, 1, 1).data];
    drawLayer(img, img.naturalWidth, img.naturalHeight, false);
    const centerPix = [...ctx.getImageData(w / 2, h / 2, 1, 1).data];

    return {
      videoReadyState,
      advanced,
      videoSize: [video.videoWidth, video.videoHeight],
      imgSize: [img.naturalWidth, img.naturalHeight],
      topPix, // expect red-ish (base video visible where the overlay doesn't reach)
      centerPix, // expect green (image overlay on top)
    };
  },
  { base },
);

await browser.close();
server.close();

console.log(JSON.stringify(result, null, 2));

// assertions
const isRed = result.topPix[0] > 150 && result.topPix[1] < 90 && result.topPix[2] < 90;
const isGreen = result.centerPix[1] > 150 && result.centerPix[0] < 120 && result.centerPix[2] < 120;
const ok = result.videoReadyState >= 2 && result.advanced && isRed && isGreen;
console.log("\nvideo decoded + played:", result.videoReadyState >= 2 && result.advanced);
console.log("base video visible (red top):", isRed);
console.log("image overlay on top (green center):", isGreen);
console.log(ok ? "\n✅ COMPOSITOR OK" : "\n❌ COMPOSITOR FAILED");
process.exit(ok ? 0 : 1);
