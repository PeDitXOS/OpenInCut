/**
 * Parity harness for the REAL webview compositor (src/engine/compositor.ts):
 * loads the module through the vite dev server in Chromium, feeds it real
 * decoded video (VP9, decodable in the bundled Chromium), and asserts pixels
 * for every export rule the canvas must reproduce:
 *
 *   1. base fill        — base-track clip fills the canvas (contain)
 *   2. gap PiP          — base gap keeps upper layers native-size over black
 *   3. scale cap        — upper layer effective scale = min(s, cw/sw, ch/sh)
 *   4. crop             — transform crop cuts the source, not the canvas
 *   5. chroma key       — core.chroma_key keys the green out per pixel
 *   6. crossfade        — base-track transition blends inside the xfade window
 *   7. text style       — titles draw with the clip's TextStyle (color/size)
 *   8. generator base   — a base-track generator fills the canvas like export
 *
 * Usage: node scripts/verify-preview-parity.mjs
 */
import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { mkdirSync, readFileSync } from "node:fs";
import { join, extname } from "node:path";
import { chromium, webkit } from "playwright";

// The Tauri webview on macOS IS WebKit: run there by default so what we prove
// is what the app actually does. `--chromium` cross-checks the other engine.
const engineName = process.argv.includes("--chromium") ? "chromium" : "webkit";
const engineLauncher = engineName === "chromium" ? chromium : webkit;

const appUrl = "http://localhost:5175";
const mediaDir = join(process.env.TMPDIR ?? "/tmp", "ue-parity-media");
mkdirSync(mediaDir, { recursive: true });

// ---- test media (VP9: open codec, decodable by the bundled Chromium) ----
const gen = (name, filter, extra = []) => {
  const out = join(mediaDir, name);
  execFileSync("ffmpeg", [
    "-y", "-v", "error",
    "-f", "lavfi", "-i", filter,
    "-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8",
    ...extra,
    out,
  ]);
  return name;
};
console.log("generating test media…");
gen("red.webm", "color=red:size=640x360:rate=30:duration=3");
gen("blue.webm", "color=blue:size=640x360:rate=30:duration=3");
gen("bigblue.webm", "color=blue:size=3840x2160:rate=5:duration=1");
gen("green.webm", "color=0x00FF00:size=640x360:rate=30:duration=1");

// hard black|white vertical split: a blur is unmistakable across the seam
execFileSync("ffmpeg", [
  "-y", "-v", "error",
  "-f", "lavfi", "-i", "color=black:s=320x360",
  "-f", "lavfi", "-i", "color=white:s=320x360",
  "-filter_complex", "[0:v][1:v]hstack=inputs=2,format=yuv420p",
  "-frames:v", "1", join(mediaDir, "split.png"),
]);
execFileSync("ffmpeg", [
  "-y", "-v", "error", "-loop", "1", "-i", join(mediaDir, "split.png"), "-t", "2",
  "-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-pix_fmt", "yuv420p",
  join(mediaDir, "split.webm"),
]);

// static gradient (VP9 decodes bit-exactly everywhere → deterministic eq test)
const EQ = { brightness: 0.12, contrast: 1.35, saturation: 1.6, gamma: 1.25 };
execFileSync("ffmpeg", [
  "-y", "-v", "error",
  "-f", "lavfi", "-i", "gradients=s=640x360:n=4:speed=0.000001:duration=1",
  "-frames:v", "1", join(mediaDir, "grad.png"),
]);
execFileSync("ffmpeg", [
  "-y", "-v", "error", "-loop", "1", "-i", join(mediaDir, "grad.png"), "-t", "2",
  "-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-pix_fmt", "yuv420p",
  join(mediaDir, "grad.webm"),
]);
// export-exact reference: the clip's eq chain, then the base norm, then the
// preview downscale — the very filters the export/paused-frame path runs
const NORM = "scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,scale=960:-2";
execFileSync("ffmpeg", [
  "-y", "-v", "error", "-ss", "1", "-i", join(mediaDir, "grad.webm"), "-frames:v", "1",
  "-vf",
  `eq=brightness=${EQ.brightness}:contrast=${EQ.contrast}:saturation=${EQ.saturation}:gamma=${EQ.gamma},${NORM}`,
  join(mediaDir, "ref_eq.png"),
]);
// same frame with NO effect: the webview's own video→RGB decode differs from
// ffmpeg's, so this is the floor of anything measured through decoded video.
execFileSync("ffmpeg", [
  "-y", "-v", "error", "-ss", "1", "-i", join(mediaDir, "grad.webm"), "-frames:v", "1",
  "-vf", NORM,
  join(mediaDir, "ref_plain.png"),
]);
// ffmpeg's eq applied to the PNG ITSELF: same pixels in, so comparing our port
// against this measures the eq maths and nothing else.
//
// yuv444p is forced on purpose. `eq` only accepts YUV, so with an RGB input
// ffmpeg silently inserts a yuv420p conversion — and the CHROMA SUBSAMPLING
// that entails was landing in the "error" of our port, which works at full
// chroma. The test was grading the pixel format, not the maths.
execFileSync("ffmpeg", [
  "-y", "-v", "error", "-i", join(mediaDir, "grad.png"), "-frames:v", "1",
  "-vf",
  `format=yuv444p,eq=brightness=${EQ.brightness}:contrast=${EQ.contrast}:saturation=${EQ.saturation}:gamma=${EQ.gamma},format=rgb24`,
  join(mediaDir, "ref_eq_raw.png"),
]);

// ---- media server with range support (like the asset protocol) ----
const TYPES = { ".webm": "video/webm", ".png": "image/png" };
const server = createServer((req, res) => {
  try {
    const name = decodeURIComponent(req.url.split("?")[0]);
    const buf = readFileSync(join(mediaDir, name));
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
const mediaBase = `http://localhost:${server.address().port}`;

// ---- vite dev server (reused if already running) ----
let devServer = null;
const reachable = async () => {
  try {
    return (await fetch(appUrl, { signal: AbortSignal.timeout(1500) })).ok;
  } catch {
    return false;
  }
};
if (!(await reachable())) {
  console.log("starting vite…");
  devServer = spawn("npm", ["run", "dev"], { stdio: "ignore" });
  for (let i = 0; i < 40 && !(await reachable()); i++) await new Promise((r) => setTimeout(r, 500));
  if (!(await reachable())) throw new Error("could not start the dev server");
}
const cleanup = () => {
  devServer?.kill("SIGKILL");
  server.close();
};
process.on("exit", cleanup);

console.log(`engine: ${engineName}`);
const browser = await engineLauncher.launch();
const page = await browser.newPage();
page.on("pageerror", (e) => console.log("[pageerror]", e.message));
await page.goto(appUrl, { waitUntil: "networkidle" });

const results = await page.evaluate(async ({ mediaBase }) => {
  const comp = await import("/src/engine/compositor.ts");

  // ---- fixtures ----------------------------------------------------------
  let idn = 0;
  const id = () => `id${++idn}`;
  const DEFAULT_TRANSFORM = () => ({
    position: [0, 0], scale: [1, 1], rotation: 0, crop: [0, 0, 0, 0],
    opacity: 1, flip_h: false, flip_v: false,
  });
  const DEFAULT_AUDIO = () => ({
    gain_db: 0, pan: 0, fade_in_us: 0, fade_out_us: 0, muted: false, denoise: false,
  });
  const asset = (name, w, h, durS) => ({
    id: id(), kind: "video", path: `${mediaBase}/${name}`, content_hash: name,
    probe: { duration_us: durS * 1e6, fps: [30, 1], width: w, height: h, rotation: 0, vcodec: "vp9", acodec: null, audio_channels: 0, vfr: false },
    proxy: null, audio_conform: null, peaks: null, thumbnails: null, transcript: null, offline: false,
  });
  const mediaClip = (a, srcInS, srcOutS, startS, extra = {}) => ({
    id: id(),
    payload: { type: "media", asset_id: a.id, src_in: srcInS * 1e6, src_out: srcOutS * 1e6 },
    start: startS * 1e6, duration: (srcOutS - srcInS) * 1e6, speed: 1,
    effects: [], transform: DEFAULT_TRANSFORM(), audio: DEFAULT_AUDIO(),
    transition_in: null, label_color: null, group: null, name: null,
    ...extra,
  });
  const track = (name, clips) => ({
    id: id(), kind: "video", name, muted: false, solo: false, locked: false, volume_db: 0, clips,
  });
  const project = (assets, tracks) => {
    const seq = { id: id(), name: "Main", resolution: [1920, 1080], fps: [30, 1], sample_rate: 48000, tracks, markers: [] };
    return {
      schema_version: 1, id: id(), name: "t", created_at: "",
      settings: { whisper_language: "auto", whisper_model: "base", autosave_secs: 60 },
      assets, transcripts: [], avatars: [], sequences: [seq], active_sequence: seq.id,
    };
  };

  // ---- frame sources over native <video> (solid colors: any frame works) --
  const videoEls = new Map();
  const getVideoEl = (url) => {
    if (videoEls.has(url)) return videoEls.get(url);
    const p = new Promise((res, rej) => {
      const v = document.createElement("video");
      v.crossOrigin = "anonymous";
      v.muted = true;
      v.preload = "auto";
      v.src = url;
      v.onloadeddata = () => res(v);
      v.onerror = () => rej(new Error(`video failed: ${url}`));
    });
    videoEls.set(url, p);
    return p;
  };
  const mkSources = (proj) => ({
    async video(assetId, _clipId, timeSec) {
      const a = proj.assets.find((x) => x.id === assetId);
      const v = await getVideoEl(a.path);
      const t = Math.max(0, Math.min(timeSec, (v.duration || 1) - 0.05));
      if (Math.abs(v.currentTime - t) > 0.05) {
        v.currentTime = t;
        await new Promise((r) => (v.onseeked = r));
      }
      return { source: v, sw: v.videoWidth, sh: v.videoHeight };
    },
    image() {
      return null;
    },
  });

  const W = 960, H = 540;
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  const px = (x, y) => [...ctx.getImageData(x, y, 1, 1).data];
  const isRed = (p) => p[0] > 150 && p[1] < 100 && p[2] < 100;
  const isBlue = (p) => p[2] > 150 && p[0] < 100;
  const isBlack = (p) => p[0] < 40 && p[1] < 40 && p[2] < 40;
  const results = {};
  const run = async (proj, tS) => {
    const seq = proj.sequences[0];
    await comp.compositeFrame(ctx, W, H, proj, seq, tS * 1e6, mkSources(proj));
  };

  // 1. base fill: red on V1 fills the whole canvas
  {
    const red = asset("red.webm", 640, 360, 3);
    const proj = project([red], [track("V1", [mediaClip(red, 0, 3, 0)])]);
    await run(proj, 1);
    results.baseFill = { corner: px(20, 20), center: px(480, 270), pass: isRed(px(20, 20)) && isRed(px(480, 270)) };
  }

  // 2. gap PiP: base gap at t=2 → upper layer native-size over black
  {
    const red = asset("red.webm", 640, 360, 3);
    const blue = asset("blue.webm", 640, 360, 3);
    const proj = project(
      [red, blue],
      [track("V1", [mediaClip(red, 0, 1, 0)]), track("V2", [mediaClip(blue, 0, 3, 0)])],
    );
    await run(proj, 2);
    const corner = px(20, 20), center = px(480, 270), outside = px(300, 270), inside = px(340, 270);
    results.gapPiP = {
      corner, center, outside, inside,
      pass: isBlack(corner) && isBlue(center) && isBlack(outside) && isBlue(inside),
    };
  }

  // 3. scale cap: 4K layer at s=0.5 fills the canvas (min(s, cw/sw, ch/sh))
  {
    const red = asset("red.webm", 640, 360, 3);
    const big = asset("bigblue.webm", 3840, 2160, 1);
    const clip = mediaClip(big, 0, 1, 0);
    clip.transform.scale = [0.5, 0.5];
    const proj = project([red, big], [track("V1", [mediaClip(red, 0, 3, 0)]), track("V2", [clip])]);
    await run(proj, 0.5);
    results.scaleCap = { corner: px(20, 20), pass: isBlue(px(20, 20)) };
  }

  // 4. crop: l=r=0.25 shows the middle half of the layer only
  {
    const red = asset("red.webm", 640, 360, 3);
    const blue = asset("blue.webm", 640, 360, 3);
    const clip = mediaClip(blue, 0, 3, 0);
    clip.transform.crop = [0.25, 0, 0.25, 0];
    const proj = project([red, blue], [track("V1", [mediaClip(red, 0, 3, 0)]), track("V2", [clip])]);
    await run(proj, 0.5);
    const center = px(480, 270), left = px(390, 270);
    results.crop = { center, left, pass: isBlue(center) && isRed(left) };
  }

  // 5. chroma key: green layer keyed out → red base shows through
  {
    const red = asset("red.webm", 640, 360, 3);
    const green = asset("green.webm", 640, 360, 1);
    const clip = mediaClip(green, 0, 1, 0);
    clip.effects = [{
      effect_id: "core.chroma_key", enabled: true,
      params: { similarity: 0.3, blend: 0.1, despill: 0.5 },
      color_params: { key_color: "#00ff00" },
    }];
    const proj = project([red, green], [track("V1", [mediaClip(red, 0, 3, 0)]), track("V2", [clip])]);
    await run(proj, 0.5);
    results.chroma = { center: px(480, 270), pass: isRed(px(480, 270)) };
  }

  // 6. crossfade on the base track: pure A → blend at the cut → pure B
  {
    const red = asset("red.webm", 640, 360, 3);
    const blue = asset("blue.webm", 640, 360, 3);
    const a = mediaClip(red, 0, 2, 0);
    const b = mediaClip(blue, 1, 2, 2);
    b.transition_in = { effect_id: "core.crossfade", duration: 1e6, params: {} };
    const proj = project([red, blue], [track("V1", [a, b])]);
    await run(proj, 1.4);
    const before = px(480, 270);
    await run(proj, 2.0);
    const mid = px(480, 270);
    await run(proj, 2.6);
    const after = px(480, 270);
    const blended = mid[0] > 60 && mid[0] < 200 && mid[2] > 60 && mid[2] < 200;
    results.crossfade = { before, mid, after, pass: isRed(before) && blended && isBlue(after) };
  }

  // 7. text style: a green 120px title paints green somewhere on its row
  {
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, W, H);
    const textClip = {
      id: id(),
      payload: {
        type: "text", content: "TITLE",
        style: { font: "sans-serif", size: 120, color: "#00ff00", bg: null, stroke_color: null, stroke_width: 0, highlight_color: null, x_offset: 0, y_offset: 0, align: "center" },
      },
      start: 0, duration: 3e6, speed: 1, effects: [], transform: DEFAULT_TRANSFORM(),
      audio: DEFAULT_AUDIO(), transition_in: null, label_color: null, group: null, name: null,
    };
    comp.drawOverlays(ctx, W, H, [textClip], [], false);
    const row = ctx.getImageData(0, 270, W, 1).data;
    let green = 0;
    for (let i = 0; i < row.length; i += 4) if (row[i + 1] > 150 && row[i] < 100) green++;
    results.textStyle = { greenPixelsOnCenterRow: green, pass: green > 20 };
  }

  // 9. color_correct: the canvas eq port must reproduce ffmpeg's eq.
  //    Driven from the PNG, not from a decoded video: the webview's video→RGB
  //    decode is noisy (the baseline wandered 14→22 between runs) and would
  //    drown the very thing being measured. Here both sides start from the
  //    exact same pixels, so any difference IS the eq port's.
  {
    const loadImg = async (name) => {
      const im = new Image();
      im.crossOrigin = "anonymous";
      im.src = `${mediaBase}/${name}`;
      await new Promise((res, rej) => { im.onload = res; im.onerror = rej; });
      return im;
    };
    const px = async (name) => {
      const im = await loadImg(name);
      const c = document.createElement("canvas");
      c.width = im.naturalWidth;
      c.height = im.naturalHeight;
      const cx = c.getContext("2d", { willReadFrequently: true });
      cx.drawImage(im, 0, 0);
      return cx.getImageData(0, 0, c.width, c.height);
    };

    const src = await px("grad.png");           // the source frame
    const ref = await px("ref_eq_raw.png");     // ffmpeg's eq of that frame
    const mine = new ImageData(new Uint8ClampedArray(src.data), src.width, src.height);
    comp.applyEqForTest(mine.data, {
      kind: "eq", brightness: 0.12, contrast: 1.35, saturation: 1.6, gamma: 1.25,
    });

    let sum = 0, n = 0, worst = 0;
    for (let i = 0; i < mine.data.length; i += 4) {
      const d = Math.max(
        Math.abs(mine.data[i] - ref.data[i]),
        Math.abs(mine.data[i + 1] - ref.data[i + 1]),
        Math.abs(mine.data[i + 2] - ref.data[i + 2]),
      );
      sum += d; n++;
      if (d > worst) worst = d;
    }
    const mean = sum / n;
    results.eqExact = {
      meanAbsDiff: Math.round(mean * 100) / 100,
      worstChannelDiff: worst,
      pixels: n,
      pass: mean < 3 && worst < 24,
    };
  }

  // 9b. the same eq through the FULL compositor, over decoded video: this can
  //     only be as good as the webview's own decode, so it is reported (not
  //     asserted tightly) to keep the real-world floor visible.
  {
    const grad = asset("grad.webm", 640, 360, 2);
    const loadRef = async (name) => {
      const ref = new Image();
      ref.crossOrigin = "anonymous";
      ref.src = `${mediaBase}/${name}`;
      await new Promise((res, rej) => { ref.onload = res; ref.onerror = rej; });
      const rc = document.createElement("canvas");
      rc.width = W; rc.height = H;
      const rctx = rc.getContext("2d", { willReadFrequently: true });
      rctx.drawImage(ref, 0, 0, W, H);
      return rctx.getImageData(0, 0, W, H).data;
    };
    const meanDiff = (a, b) => {
      let sum = 0, n = 0;
      for (let y = 40; y < H - 40; y += 16) {
        for (let x = 40; x < W - 40; x += 16) {
          const i = (y * W + x) * 4;
          sum += Math.max(
            Math.abs(a[i] - b[i]),
            Math.abs(a[i + 1] - b[i + 1]),
            Math.abs(a[i + 2] - b[i + 2]),
          );
          n++;
        }
      }
      return sum / n;
    };

    // baseline: no effect at all vs ffmpeg's plain render
    const plainClip = mediaClip(grad, 0, 2, 0);
    await run(project([grad], [track("V1", [plainClip])]), 1);
    const baseline = meanDiff(ctx.getImageData(0, 0, W, H).data, await loadRef("ref_plain.png"));

    // with eq, vs ffmpeg's eq render
    const eqClip = mediaClip(grad, 0, 2, 0);
    eqClip.effects = [{
      effect_id: "core.color_correct", enabled: true,
      params: { brightness: 0.12, contrast: 1.35, saturation: 1.6, gamma: 1.25 },
      color_params: {},
    }];
    await run(project([grad], [track("V1", [eqClip])]), 1);
    const withEq = meanDiff(ctx.getImageData(0, 0, W, H).data, await loadRef("ref_eq.png"));

    results.eqThroughDecodedVideo = {
      webviewDecodeFloor: Math.round(baseline * 100) / 100,
      withEq: Math.round(withEq * 100) / 100,
      note: "the floor is the webview's video decode, not the eq port (see eqExact)",
      pass: true, // reported, not gated: it measures the decoder, not our maths
    };
  }

  // 10. core.vertical_fill: the cover background is REALLY blurred (the seam
  //     of a hard black|white source turns into a gradient), and the
  //     width-fit foreground sits sharp on top.
  {
    const split = asset("split.webm", 640, 360, 2);
    const clip = mediaClip(split, 0, 2, 0);
    clip.effects = [{
      effect_id: "core.vertical_fill", enabled: true,
      params: { width: 1080, height: 1920, blur: 20 }, color_params: {},
    }];
    const proj = project([split], [track("V1", [clip])]);
    proj.sequences[0].resolution = [1080, 1920];
    // vertical canvas for this case
    const VW = 405, VH = 720;
    canvas.width = VW; canvas.height = VH;
    await comp.compositeFrame(ctx, VW, VH, proj, proj.sequences[0], 1e6, mkSources(proj));
    // y=100 is background only (the fg band spans y≈246..474)
    const row = ctx.getImageData(0, 100, VW, 1).data;
    let mids = 0;
    for (let x = VW / 2 - 14; x < VW / 2 + 14; x++) {
      const v = row[Math.round(x) * 4];
      if (v > 60 && v < 195) mids++; // neither black nor white → blurred
    }
    // and the foreground stays sharp: its own seam has almost no mid values
    const fgRow = ctx.getImageData(0, 360, VW, 1).data;
    let fgMids = 0;
    for (let x = VW / 2 - 14; x < VW / 2 + 14; x++) {
      const v = fgRow[Math.round(x) * 4];
      if (v > 60 && v < 195) fgMids++;
    }
    results.verticalFillBlur = {
      blurredBackgroundMidPixels: mids,
      sharpForegroundMidPixels: fgMids,
      pass: mids >= 8 && fgMids <= 4,
    };
    canvas.width = W; canvas.height = H;
  }

  // 11. transform APPLIES on top of vertical_fill (the bug: the old code
  //     returned early and only opacity survived). scale 0.5 → the filled
  //     frame shrinks to the middle and the corners go black.
  {
    const split = asset("split.webm", 640, 360, 2);
    const clip = mediaClip(split, 0, 2, 0);
    clip.effects = [{
      effect_id: "core.vertical_fill", enabled: true,
      params: { width: 1080, height: 1920, blur: 20 }, color_params: {},
    }];
    clip.transform.scale = [0.5, 0.5];
    const proj = project([split], [track("V1", [clip])]);
    proj.sequences[0].resolution = [1080, 1920];
    const VW = 405, VH = 720;
    canvas.width = VW; canvas.height = VH;
    await comp.compositeFrame(ctx, VW, VH, proj, proj.sequences[0], 1e6, mkSources(proj));
    const corner = [...ctx.getImageData(12, 12, 1, 1).data];
    const centre = [...ctx.getImageData(VW / 2, VH / 2, 1, 1).data];
    results.verticalFillTransform = {
      corner, centre,
      pass: isBlack(corner) && !isBlack(centre),
    };
    canvas.width = W; canvas.height = H;
  }

  // 13. WRAP: a caption too wide for the frame must break into lines, the
  //     block must stay centred on y_offset, and no line may spill past the
  //     usable width. Same algorithm as graph.rs (per-word measurement).
  {
    const style = {
      font: "sans-serif", size: 80, color: "#ffffff", bg: null, stroke_color: null,
      stroke_width: 0, highlight_color: null, x_offset: 0, y_offset: 0,
      align: "center", line_height: 1.2,
    };
    const textClip = {
      id: id(),
      payload: { type: "text", content: "esta frase es demasiado larga para caber en una sola linea del video", style },
      start: 0, duration: 3e6, speed: 1, effects: [], transform: DEFAULT_TRANSFORM(),
      audio: DEFAULT_AUDIO(), transition_in: null, label_color: null, group: null, name: null,
    };
    const VW = 405, VH = 720; // 1080x1920 canvas
    canvas.width = VW;
    canvas.height = VH;
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, VW, VH);
    comp.drawOverlays(ctx, VW, VH, [textClip], [], false);

    const d = ctx.getImageData(0, 0, VW, VH).data;
    const rows = [];
    let widest = 0;
    for (let y = 0; y < VH; y++) {
      let minX = -1, maxX = -1;
      for (let x = 0; x < VW; x++) {
        if (d[(y * VW + x) * 4] > 120) {
          if (minX < 0) minX = x;
          maxX = x;
        }
      }
      if (minX >= 0) {
        rows.push(y);
        widest = Math.max(widest, maxX - minX + 1);
      }
    }
    let lines = 0;
    rows.forEach((y, i) => {
      if (i === 0 || y > rows[i - 1] + 2) lines++;
    });
    const mid = rows.length ? (rows[0] + rows[rows.length - 1]) / 2 : 0;
    results.captionWrap = {
      lines,
      widestLinePx: widest,
      usableWidthPx: Math.round(VW * 0.86),
      blockCentre: Math.round(mid),
      frameCentre: VH / 2,
      pass: lines >= 2 && widest <= Math.round(VW * 0.86) + 2 && Math.abs(mid - VH / 2) < 30,
    };
    canvas.width = W;
    canvas.height = H;
  }

  // 12. PERFORMANCE: a vertical_fill clip (the heaviest common case: cover
  //     background + blur + sharp foreground) must composite fast enough for
  //     real playback. The software blur once ran per-frame JS passes over a
  //     full-size image and dropped this to ~1 fps.
  {
    const split = asset("split.webm", 640, 360, 2);
    const clip = mediaClip(split, 0, 2, 0);
    clip.effects = [{
      effect_id: "core.vertical_fill", enabled: true,
      params: { width: 1080, height: 1920, blur: 20 }, color_params: {},
    }];
    const proj = project([split], [track("V1", [clip])]);
    proj.sequences[0].resolution = [1080, 1920];
    const VW = 405, VH = 720;
    // emulate the real preview: a DPR-2 canvas (device px = 810×1440)
    canvas.width = VW * 2;
    canvas.height = VH * 2;
    ctx.setTransform(2, 0, 0, 2, 0, 0);
    const seq = proj.sequences[0];
    const src = mkSources(proj);
    await comp.compositeFrame(ctx, VW, VH, proj, seq, 0, src); // warm up
    const t0 = performance.now();
    const N = 30;
    for (let i = 0; i < N; i++) {
      await comp.compositeFrame(ctx, VW, VH, proj, seq, i * 33_000, src);
    }
    const msPerFrame = (performance.now() - t0) / N;
    results.playbackSpeed = {
      msPerFrame: Math.round(msPerFrame * 10) / 10,
      fps: Math.round(1000 / msPerFrame),
      pass: msPerFrame < 33, // ≥ 30 fps
    };
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    canvas.width = W;
    canvas.height = H;
  }

  // 8. generator on the base track fills the canvas (export: lavfi + norm)
  {
    const genClip = {
      id: id(),
      payload: { type: "generator", generator_id: "core.solid", params: { width: 640, height: 360 }, color_params: { color: "#33cc66" } },
      start: 0, duration: 3e6, speed: 1, effects: [], transform: DEFAULT_TRANSFORM(),
      audio: DEFAULT_AUDIO(), transition_in: null, label_color: null, group: null, name: null,
    };
    const proj = project([], [track("V1", [genClip])]);
    await run(proj, 1);
    const corner = px(20, 20);
    results.generatorBase = { corner, pass: corner[1] > 120 && corner[0] < 120 };
  }

  return results;
}, { mediaBase });

// ---------------------------------------------------------------------------
// WebKit simulation: the Tauri webview on macOS silently ignores
// ctx.filter, which is why the blurred vertical-fill background rendered SHARP
// during playback. Neuter ctx.filter the same way and prove the compositor's
// own blur still produces a blurred background.
// ---------------------------------------------------------------------------
const wkPage = await browser.newPage();
wkPage.on("pageerror", (e) => console.log("[pageerror:webkit-sim]", e.message));
await wkPage.addInitScript(() => {
  Object.defineProperty(CanvasRenderingContext2D.prototype, "filter", {
    get: () => "none", // exactly what a non-supporting engine reports
    set: () => {},
    configurable: true,
  });
});
await wkPage.goto(appUrl, { waitUntil: "networkidle" });

// PERFORMANCE under the REAL app's conditions. The previous benchmark ran with
// ctx.filter available (Playwright's WebKit is newer than the WKWebView that
// Tauri embeds) and a small 640x360 source, so it timed the GPU path the app
// never takes: it said 297 fps while the app crawled at 6. This one stubs
// ctx.filter out AND uses a 1920x1080 source into a 1080x1920 canvas at DPR 2.
results.playbackSpeedRealConditions = await wkPage.evaluate(
  async ({ mediaBase }) => {
    const comp = await import("/src/engine/compositor.ts");
    if (comp.canvasFilterSupported()) return { pass: false, why: "ctx.filter stub failed" };
    const VW = 560, VH = 996, DPR = 2;
    const canvas = document.createElement("canvas");
    canvas.width = VW * DPR;
    canvas.height = VH * DPR;
    const ctx = canvas.getContext("2d");
    ctx.setTransform(DPR, 0, 0, DPR, 0, 0);
    const video = document.createElement("video");
    video.crossOrigin = "anonymous";
    video.muted = true;
    video.loop = true;
    video.src = `${mediaBase}/big1080.webm`;
    await new Promise((res, rej) => { video.onloadeddata = res; video.onerror = () => rej(new Error("video failed")); });
    await video.play();
    const id = (n) => `p${n}`;
    const asset = {
      id: id(1), kind: "video", path: "x", content_hash: "h",
      probe: { duration_us: 3e6, fps: [30, 1], width: 1920, height: 1080, rotation: 0, vcodec: "vp9", acodec: null, audio_channels: 0, vfr: false },
      proxy: null, audio_conform: null, peaks: null, thumbnails: null, transcript: null, offline: false,
    };
    const clip = {
      id: id(2),
      payload: { type: "media", asset_id: asset.id, src_in: 0, src_out: 3e6 },
      start: 0, duration: 3e6, speed: 1,
      effects: [{ effect_id: "core.vertical_fill", enabled: true, params: { width: 1080, height: 1920, blur: 20 }, color_params: {} }],
      transform: { position: [0, 0], scale: [1, 1], rotation: 0, crop: [0, 0, 0, 0], opacity: 1, flip_h: false, flip_v: false },
      audio: { gain_db: 0, pan: 0, fade_in_us: 0, fade_out_us: 0, muted: false, denoise: false },
      transition_in: null, label_color: null, group: null, name: null,
    };
    const seq = { id: id(3), name: "M", resolution: [1080, 1920], fps: [30, 1], sample_rate: 48000, markers: [],
      tracks: [{ id: id(4), kind: "video", name: "V1", muted: false, solo: false, locked: false, volume_db: 0, clips: [clip] }] };
    const proj = { schema_version: 1, id: id(5), name: "perf", created_at: "",
      settings: { whisper_language: "auto", whisper_model: "base", autosave_secs: 60 },
      assets: [asset], transcripts: [], avatars: [], sequences: [seq], active_sequence: seq.id };
    const sources = { async video() { return { source: video, sw: 1920, sh: 1080 }; }, image: () => null };
    // drawImage is QUEUED on the GPU: without forcing a readback each frame we
    // would time only the command submission and report a fantasy number.
    const flush = () => ctx.getImageData(0, 0, 1, 1);
    await comp.compositeFrame(ctx, VW, VH, proj, seq, 0, sources);
    flush();
    const N = 30;
    const t0 = performance.now();
    for (let i = 0; i < N; i++) {
      await comp.compositeFrame(ctx, VW, VH, proj, seq, i * 33000, sources);
      flush();
    }
    const ms = (performance.now() - t0) / N;
    return { msPerFrame: Math.round(ms * 10) / 10, fps: Math.round(1000 / ms),
             note: "ctx.filter stubbed + 1920x1080 source + 1080x1920 @dpr2 = the app", pass: ms < 33 };
  },
  { mediaBase },
);

results.verticalFillBlurNoCtxFilter = await wkPage.evaluate(
  async ({ mediaBase }) => {
    const comp = await import("/src/engine/compositor.ts");
    if (comp.canvasFilterSupported()) {
      return { pass: false, why: "the ctx.filter stub did not take effect" };
    }
    const VW = 405, VH = 720;
    const canvas = document.createElement("canvas");
    canvas.width = VW;
    canvas.height = VH;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });

    const video = document.createElement("video");
    video.crossOrigin = "anonymous";
    video.muted = true;
    video.src = `${mediaBase}/split.webm`;
    await new Promise((res, rej) => {
      video.onloadeddata = res;
      video.onerror = () => rej(new Error("video failed"));
    });

    const id = (n) => `w${n}`;
    const asset = {
      id: id(1), kind: "video", path: `${mediaBase}/split.webm`, content_hash: "s",
      probe: { duration_us: 2e6, fps: [30, 1], width: 640, height: 360, rotation: 0, vcodec: "vp9", acodec: null, audio_channels: 0, vfr: false },
      proxy: null, audio_conform: null, peaks: null, thumbnails: null, transcript: null, offline: false,
    };
    const clip = {
      id: id(2),
      payload: { type: "media", asset_id: asset.id, src_in: 0, src_out: 2e6 },
      start: 0, duration: 2e6, speed: 1,
      effects: [{ effect_id: "core.vertical_fill", enabled: true, params: { width: 1080, height: 1920, blur: 20 }, color_params: {} }],
      transform: { position: [0, 0], scale: [1, 1], rotation: 0, crop: [0, 0, 0, 0], opacity: 1, flip_h: false, flip_v: false },
      audio: { gain_db: 0, pan: 0, fade_in_us: 0, fade_out_us: 0, muted: false, denoise: false },
      transition_in: null, label_color: null, group: null, name: null,
    };
    const seq = { id: id(3), name: "M", resolution: [1080, 1920], fps: [30, 1], sample_rate: 48000, markers: [], tracks: [{ id: id(4), kind: "video", name: "V1", muted: false, solo: false, locked: false, volume_db: 0, clips: [clip] }] };
    const proj = {
      schema_version: 1, id: id(5), name: "wk", created_at: "",
      settings: { whisper_language: "auto", whisper_model: "base", autosave_secs: 60 },
      assets: [asset], transcripts: [], avatars: [], sequences: [seq], active_sequence: seq.id,
    };
    const sources = {
      async video() {
        if (video.currentTime < 0.4) {
          video.currentTime = 0.5;
          await new Promise((r) => (video.onseeked = r));
        }
        return { source: video, sw: video.videoWidth, sh: video.videoHeight };
      },
      image: () => null,
    };
    await comp.compositeFrame(ctx, VW, VH, proj, seq, 1e6, sources);

    const row = ctx.getImageData(0, 100, VW, 1).data;
    let mids = 0;
    for (let x = VW / 2 - 14; x < VW / 2 + 14; x++) {
      const v = row[Math.round(x) * 4];
      if (v > 60 && v < 195) mids++;
    }
    return {
      ctxFilterSupported: comp.canvasFilterSupported(),
      blurredBackgroundMidPixels: mids,
      pass: mids >= 8,
    };
  },
  { mediaBase },
);

await browser.close();
cleanup();

console.log(JSON.stringify(results, null, 2));
const failed = Object.entries(results).filter(([, r]) => !r.pass);
for (const [name, r] of Object.entries(results)) {
  console.log(`${r.pass ? "✅" : "❌"} ${name}`);
}
process.exit(failed.length ? 1 : 0);
